#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype,
    Address, Env, String, Symbol, Vec,
    Val, IntoVal,
};
use soroban_sdk::token::Client as TokenClient;

/* -------------------- STORAGE KEYS -------------------- */

const ADMIN_KEY: Symbol = Symbol::short("ADMIN");
const NFT_KEY: Symbol = Symbol::short("NFT");
const COMMITMENTS_KEY: Symbol = Symbol::short("COMMS");

/* -------------------- DATA TYPES -------------------- */
    contract, contracterror, contractimpl, contracttype, log, token, symbol_short, Address, Env, IntoVal, String,
    Symbol, Vec,
};
use shared_utils::{SafeMath, TimeUtils, Validation, RateLimiter};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CommitmentError {
    InvalidDuration = 1,
    InvalidMaxLossPercent = 2,
    InvalidCommitmentType = 3,
    InvalidAmount = 4,
    InsufficientBalance = 5,
    TransferFailed = 6,
    MintingFailed = 7,
    CommitmentNotFound = 8,
    Unauthorized = 9,
    AlreadyInitialized = 10,
    ReentrancyDetected = 11,
}

#[contracttype]
#[derive(Clone)]
pub struct CommitmentCreatedEvent {
    pub commitment_id: String,
    pub owner: Address,
    pub amount: i128,
    pub asset_address: Address,
    pub nft_token_id: u32,
    pub rules: CommitmentRules,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitmentRules {
    pub duration_days: u32,
    pub max_loss_percent: u32,
    pub commitment_type: String,
    pub early_exit_penalty: u32,
    pub min_fee_threshold: i128,
    pub grace_period_days: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Commitment {
    pub commitment_id: String,
    pub owner: Address,
    pub nft_token_id: u32,
    pub rules: CommitmentRules,
    pub amount: i128,
    pub asset_address: Address,
    pub created_at: u64,
    pub expires_at: u64,
    pub current_value: i128,
    pub status: String, // active | settled | violated | early_exit
}

/* -------------------- CONTRACT -------------------- */
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    NftContract,
    Commitment(String),        // commitment_id -> Commitment
    OwnerCommitments(Address), // owner -> Vec<commitment_id>
    TotalCommitments,          // counter
    ReentrancyGuard,           // reentrancy protection flag
    TotalValueLocked,          // aggregate value locked across active commitments
}

/// Transfer assets from owner to contract
fn transfer_assets(e: &Env, from: &Address, to: &Address, asset_address: &Address, amount: i128) {
    let token_client = token::Client::new(e, asset_address);

    // Check balance first
    let balance = token_client.balance(from);
    if balance < amount {
        log!(e, "Insufficient balance: {} < {}", balance, amount);
        panic!("Insufficient balance");
    }

    // Transfer tokens (fails transaction if unsuccessful)
    token_client.transfer(from, to, &amount);
}

/// Helper function to call NFT contract mint function
fn call_nft_mint(
    e: &Env,
    nft_contract: &Address,
    owner: &Address,
    commitment_id: &String,
    duration_days: u32,
    max_loss_percent: u32,
    commitment_type: &String,
    initial_amount: i128,
    asset_address: &Address,
) -> u32 {
    let mut args = Vec::new(e);
    args.push_back(owner.clone().into_val(e));
    args.push_back(commitment_id.clone().into_val(e));
    args.push_back(duration_days.into_val(e));
    args.push_back(max_loss_percent.into_val(e));
    args.push_back(commitment_type.clone().into_val(e));
    args.push_back(initial_amount.into_val(e));
    args.push_back(asset_address.clone().into_val(e));

    // In Soroban, contract calls return the value directly
    // Failures cause the entire transaction to fail
    e.invoke_contract::<u32>(nft_contract, &Symbol::new(e, "mint"), args)
}

// Storage helpers
fn read_commitment(e: &Env, commitment_id: &String) -> Option<Commitment> {
    e.storage()
        .instance()
        .get::<_, Commitment>(&DataKey::Commitment(commitment_id.clone()))
}

fn set_commitment(e: &Env, commitment: &Commitment) {
    e.storage()
        .instance()
        .set(&DataKey::Commitment(commitment.commitment_id.clone()), commitment);
}

fn has_commitment(e: &Env, commitment_id: &String) -> bool {
    e.storage()
        .instance()
        .has(&DataKey::Commitment(commitment_id.clone()))
}

/// Reentrancy protection helpers
fn require_no_reentrancy(e: &Env) {
    let guard: bool = e.storage()
        .instance()
        .get::<_, bool>(&DataKey::ReentrancyGuard)
        .unwrap_or(false);
    
    if guard {
        panic!("Reentrancy detected");
    }
}

fn set_reentrancy_guard(e: &Env, value: bool) {
    e.storage().instance().set(&DataKey::ReentrancyGuard, &value);
}

/// Require that the caller is the admin stored in this contract.
fn require_admin(e: &Env, caller: &Address) {
    caller.require_auth();
    let admin = e
        .storage()
        .instance()
        .get::<_, Address>(&DataKey::Admin)
        .unwrap_or_else(|| panic!("Contract not initialized"));
    if *caller != admin {
        panic!("Unauthorized: only admin can perform this action");
    }
}

#[contract]
pub struct CommitmentCoreContract;

#[contractimpl]
impl CommitmentCoreContract {

    /* ---------- INITIALIZE ---------- */

    pub fn initialize(e: Env, admin: Address, nft_contract: Address) {
        admin.require_auth();

        e.storage().instance().set(&ADMIN_KEY, &admin);
        e.storage().instance().set(&NFT_KEY, &nft_contract);

        let empty: Vec<Commitment> = Vec::new(&e);
        e.storage().instance().set(&COMMITMENTS_KEY, &empty);
    }

    /* ---------- CREATE COMMITMENT ---------- */

    /// Validate commitment rules
    /// Validate commitment rules using shared utilities
    fn validate_rules(e: &Env, rules: &CommitmentRules) {
        // Duration must be > 0
        Validation::require_valid_duration(rules.duration_days);

        // Max loss percent must be between 0 and 100
        Validation::require_valid_percent(rules.max_loss_percent);

        // Commitment type must be valid
        let valid_types = ["safe", "balanced", "aggressive"];
        Validation::require_valid_commitment_type(e, &rules.commitment_type, &valid_types);
    }

    /// Generate unique commitment ID
    fn generate_commitment_id(e: &Env, _owner: &Address) -> String {
        let _counter = e
            .storage()
            .instance()
            .get::<_, u64>(&DataKey::TotalCommitments)
            .unwrap_or(0);
        // Create a simple unique ID using counter
        // This is a simplified version - in production you might want a more robust ID generation
        String::from_str(e, "commitment_") // We'll extend this with a proper implementation later
    }

    /// Initialize the core commitment contract
    pub fn initialize(e: Env, admin: Address, nft_contract: Address) {
        // Check if already initialized
        if e.storage().instance().has(&DataKey::Admin) {
            panic!("Contract already initialized");
        }

        // Store admin and NFT contract address
        e.storage().instance().set(&DataKey::Admin, &admin);
        e.storage()
            .instance()
            .set(&DataKey::NftContract, &nft_contract);

        // Initialize total commitments counter
        e.storage()
            .instance()
            .set(&DataKey::TotalCommitments, &0u64);

        // Initialize total value locked counter
        e.storage()
            .instance()
            .set(&DataKey::TotalValueLocked, &0i128);
    }

    /// Create a new commitment
    /// 
    /// # Reentrancy Protection
    /// This function uses checks-effects-interactions pattern:
    /// 1. Checks: Validate inputs
    /// 2. Effects: Update state (commitment storage, counters)
    /// 3. Interactions: External calls (token transfer, NFT mint)
    /// Reentrancy guard prevents recursive calls.
    /// 
    /// # Formal Verification
    /// **Preconditions:**
    /// - `amount > 0`
    /// - `rules.duration_days > 0`
    /// - `rules.max_loss_percent <= 100`
    /// - `rules.commitment_type ∈ {"safe", "balanced", "aggressive"}`
    /// - Contract is initialized
    /// - `reentrancy_guard == false`
    /// 
    /// **Postconditions:**
    /// - Returns unique `commitment_id`
    /// - `get_commitment(commitment_id).owner == owner`
    /// - `get_commitment(commitment_id).amount == amount`
    /// - `get_commitment(commitment_id).status == "active"`
    /// - `get_total_commitments() == old(get_total_commitments()) + 1`
    /// - `reentrancy_guard == false`
    /// 
    /// **Invariants Maintained:**
    /// - INV-1: Total commitments consistency
    /// - INV-2: Commitment balance conservation
    /// - INV-3: Owner commitment list consistency
    /// - INV-4: Reentrancy guard invariant
    /// 
    /// **Security Properties:**
    /// - SP-1: Reentrancy protection
    /// - SP-2: Access control
    /// - SP-4: State consistency
    /// - SP-5: Token conservation
    pub fn create_commitment(
        e: Env,
        owner: Address,
        amount: i128,
        asset_address: Address,
        rules: CommitmentRules,
    ) -> String {
        owner.require_auth();

        if amount <= 0 {
            panic!("Invalid amount");
        }

        let now = e.ledger().timestamp();
        let expires_at = now + (rules.duration_days as u64 * 86400);

        let commitment_id =
            String::from_str(&e, "commitment");

        // Transfer asset into contract
        TokenClient::new(&e, &asset_address)
            .transfer(&owner, &e.current_contract_address(), &amount);

        // Mint NFT
        let nft_contract: Address =
            e.storage().instance().get(&NFT_KEY).unwrap();

        let mut mint_args = Vec::<Val>::new(&e);
        mint_args.push_back(owner.clone().into_val(&e));
        mint_args.push_back(commitment_id.clone().into_val(&e));

        let nft_token_id: u32 = e.invoke_contract(
            &nft_contract,
            &Symbol::short("mint"),
            mint_args,
        );

        let mut commitments: Vec<Commitment> =
            e.storage().instance().get(&COMMITMENTS_KEY).unwrap();

        commitments.push_back(Commitment {
            commitment_id: commitment_id.clone(),
            owner: owner.clone(),
            nft_token_id,
            rules,
            amount,
            asset_address,
            created_at: now,
            expires_at,
            current_value: amount,
            status: String::from_str(&e, "active"),
        });

        e.storage().instance().set(&COMMITMENTS_KEY, &commitments);

        e.events().publish(
            (Symbol::short("CommitmentCreated"),),
            (commitment_id.clone(), owner, amount, now),
        );

        // Reentrancy protection
        require_no_reentrancy(&e);
        set_reentrancy_guard(&e, true);

        // Rate limit: per-owner commitment creation
        let fn_symbol = symbol_short!("create");
        RateLimiter::check(&e, &owner, &fn_symbol);

        // Validate amount > 0 using shared utilities
        Validation::require_positive(amount);

        // Validate rules
        Self::validate_rules(&e, &rules);

        // Generate unique commitment ID
        let commitment_id = Self::generate_commitment_id(&e, &owner);

        // Get NFT contract address
        let nft_contract = e
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::NftContract)
            .unwrap_or_else(|| {
                set_reentrancy_guard(&e, false);
                panic!("Contract not initialized")
            });

        // CHECKS: Validate commitment doesn't already exist
        if has_commitment(&e, &commitment_id) {
            set_reentrancy_guard(&e, false);
            panic!("Commitment already exists");
        }

        // EFFECTS: Update state before external calls
        // Calculate expiration timestamp using shared utilities
        let current_timestamp = TimeUtils::now(&e);
        let expires_at = TimeUtils::calculate_expiration(&e, rules.duration_days);

        // Create commitment data
        let commitment = Commitment {
            commitment_id: commitment_id.clone(),
            owner: owner.clone(),
            nft_token_id: 0, // Will be set after NFT mint
            rules: rules.clone(),
            amount,
            asset_address: asset_address.clone(),
            created_at: current_timestamp,
            expires_at,
            current_value: amount, // Initially same as amount
            status: String::from_str(&e, "active"),
        };

        // Store commitment data (before external calls)
        set_commitment(&e, &commitment);

        // Update owner's commitment list
        let mut owner_commitments = e
            .storage()
            .instance()
            .get::<_, Vec<String>>(&DataKey::OwnerCommitments(owner.clone()))
            .unwrap_or(Vec::new(&e));
        owner_commitments.push_back(commitment_id.clone());
        e.storage().instance().set(
            &DataKey::OwnerCommitments(owner.clone()),
            &owner_commitments,
        );

        // Increment total commitments counter
        let current_total = e
            .storage()
            .instance()
            .get::<_, u64>(&DataKey::TotalCommitments)
            .unwrap_or(0);
        e.storage()
            .instance()
            .set(&DataKey::TotalCommitments, &(current_total + 1));

        // Update total value locked (aggregate)
        let current_tvl = e
            .storage()
            .instance()
            .get::<_, i128>(&DataKey::TotalValueLocked)
            .unwrap_or(0);
        e.storage()
            .instance()
            .set(&DataKey::TotalValueLocked, &(current_tvl + amount));

        // INTERACTIONS: External calls (token transfer, NFT mint)
        // Transfer assets from owner to contract
        let contract_address = e.current_contract_address();
        transfer_assets(&e, &owner, &contract_address, &asset_address, amount);

        // Mint NFT
        let nft_token_id = call_nft_mint(
            &e,
            &nft_contract,
            &owner,
            &commitment_id,
            rules.duration_days,
            rules.max_loss_percent,
            &rules.commitment_type,
            amount,
            &asset_address,
        );

        // Update commitment with NFT token ID
        let mut updated_commitment = commitment;
        updated_commitment.nft_token_id = nft_token_id;
        set_commitment(&e, &updated_commitment);

        // Clear reentrancy guard
        set_reentrancy_guard(&e, false);

        // Emit creation event
        e.events().publish(
            (symbol_short!("Created"), commitment_id.clone(), owner.clone()),
            (amount, rules, nft_token_id, e.ledger().timestamp()),
        );
        commitment_id
    }

    /* ---------- GET COMMITMENT ---------- */

    pub fn get_commitment(e: Env, commitment_id: String) -> Commitment {
        let commitments: Vec<Commitment> =
            e.storage().instance().get(&COMMITMENTS_KEY).unwrap();

        for c in commitments.iter() {
            if c.commitment_id == commitment_id {
                return c;
            }
        }

        panic!("Commitment not found");
        read_commitment(&e, &commitment_id)
            .unwrap_or_else(|| panic!("Commitment not found"))
    }

    /// Get all commitments for an owner
    pub fn get_owner_commitments(e: Env, owner: Address) -> Vec<String> {
        e.storage()
            .instance()
            .get::<_, Vec<String>>(&DataKey::OwnerCommitments(owner))
            .unwrap_or(Vec::new(&e))
    }

    /// Get total number of commitments
    pub fn get_total_commitments(e: Env) -> u64 {
        e.storage()
            .instance()
            .get::<_, u64>(&DataKey::TotalCommitments)
            .unwrap_or(0)
    }

    /// Get total value locked across all active commitments.
    pub fn get_total_value_locked(e: Env) -> i128 {
        e.storage()
            .instance()
            .get::<_, i128>(&DataKey::TotalValueLocked)
            .unwrap_or(0)
    }

    /// Get admin address
    pub fn get_admin(e: Env) -> Address {
        e.storage()
            .instance()
            .get::<_, Address>(&DataKey::Admin)
            .unwrap_or_else(|| panic!("Contract not initialized"))
    }

    /// Get NFT contract address
    pub fn get_nft_contract(e: Env) -> Address {
        e.storage()
            .instance()
            .get::<_, Address>(&DataKey::NftContract)
            .unwrap_or_else(|| panic!("Contract not initialized"))
    }

    /* ---------- UPDATE VALUE ---------- */

    pub fn update_value(e: Env, commitment_id: String, new_value: i128) {
        // Global per-function rate limit (per contract instance)
        let fn_symbol = symbol_short!("upd_val");
        let contract_address = e.current_contract_address();
        RateLimiter::check(&e, &contract_address, &fn_symbol);

        // NOTE: Authorization and value update logic can be extended here.

        // Emit value update event
        e.events().publish(
            (symbol_short!("ValUpd"), commitment_id),
            (new_value, e.ledger().timestamp()),
        );
    }

    /// Check if commitment rules are violated
    /// Returns true if any rule violation is detected (loss limit or duration)
    /// 
    /// # Formal Verification
    /// **Preconditions:**
    /// - `commitment_id` exists
    /// 
    /// **Postconditions:**
    /// - Returns `true` if `loss_percent > max_loss_percent OR current_time >= expires_at`
    /// - Returns `false` otherwise
    /// - Pure function (no state changes)
    /// 
    /// **Invariants Maintained:**
    /// - INV-2: Commitment balance conservation
    /// 
    /// **Security Properties:**
    /// - SP-4: State consistency (read-only)
    pub fn check_violations(e: Env, commitment_id: String) -> bool {
        let commitment = read_commitment(&e, &commitment_id)
            .unwrap_or_else(|| panic!("Commitment not found"));

        // Skip check if already settled or violated
        let active_status = String::from_str(&e, "active");
        if commitment.status != active_status {
            return false; // Already processed
        }

        let current_time = e.ledger().timestamp();

        // Check loss limit violation
        // Calculate loss percentage using shared utilities, but handle zero-amount
        // commitments gracefully to avoid panics. A zero-amount commitment cannot
        // meaningfully violate a loss limit, so we treat its loss percent as 0.
        let loss_percent = if commitment.amount > 0 {
            SafeMath::loss_percent(commitment.amount, commitment.current_value)
        } else {
            0
        };

        // Convert max_loss_percent (u32) to i128 for comparison
        let max_loss = commitment.rules.max_loss_percent as i128;
        let loss_violated = loss_percent > max_loss;

        // Check duration violation (expired)
        let duration_violated = current_time >= commitment.expires_at;

        let violated = loss_violated || duration_violated;

        if violated {
            // Emit violation event
            e.events().publish(
                (symbol_short!("Violated"), commitment_id),
                (symbol_short!("RuleViol"), e.ledger().timestamp()),
            );
        }

        // Return true if any violation exists
        violated
    }

    /// Get detailed violation information
    /// Returns a tuple: (has_violations, loss_violated, duration_violated, loss_percent, time_remaining)
    pub fn get_violation_details(
        e: Env,
        commitment_id: String,
    ) -> (bool, bool, bool, i128, u64) {
        let commitment = read_commitment(&e, &commitment_id)
            .unwrap_or_else(|| panic!("Commitment not found"));

        let current_time = e.ledger().timestamp();

        // Calculate loss percentage
        let loss_amount = commitment.amount - commitment.current_value;
        let loss_percent = if commitment.amount > 0 {
            (loss_amount * 100) / commitment.amount
        } else {
            0
        };

        // Check loss limit violation
        let max_loss = commitment.rules.max_loss_percent as i128;
        let loss_violated = loss_percent > max_loss;

        // Check duration violation
        let duration_violated = current_time >= commitment.expires_at;

        // Calculate time remaining (0 if expired)
        let time_remaining = if current_time < commitment.expires_at {
            commitment.expires_at - current_time
        } else {
            0
        };

        let has_violations = loss_violated || duration_violated;

        (has_violations, loss_violated, duration_violated, loss_percent, time_remaining)
    }

    /// Settle commitment at maturity
    /// 
    /// # Reentrancy Protection
    /// Uses checks-effects-interactions pattern with reentrancy guard.
    pub fn settle(e: Env, commitment_id: String) {
        // Reentrancy protection
        require_no_reentrancy(&e);
        set_reentrancy_guard(&e, true);

        // CHECKS: Get and validate commitment
        let mut commitment = read_commitment(&e, &commitment_id)
            .unwrap_or_else(|| {
                set_reentrancy_guard(&e, false);
                panic!("Commitment not found")
            });

        // Verify commitment is expired
        let current_time = e.ledger().timestamp();
        if current_time < commitment.expires_at {
            set_reentrancy_guard(&e, false);
            panic!("Commitment has not expired yet");
        }

        // Verify commitment is active
        let active_status = String::from_str(&e, "active");
        if commitment.status != active_status {
            set_reentrancy_guard(&e, false);
            panic!("Commitment is not active");
        }

        // EFFECTS: Update state before external calls
        let settlement_amount = commitment.current_value;
        commitment.status = String::from_str(&e, "settled");
        set_commitment(&e, &commitment);

        // Decrease total value locked
        let current_tvl = e
            .storage()
            .instance()
            .get::<_, i128>(&DataKey::TotalValueLocked)
            .unwrap_or(0);
        let new_tvl = current_tvl - settlement_amount;
        e.storage()
            .instance()
            .set(&DataKey::TotalValueLocked, &new_tvl);

        // INTERACTIONS: External calls (token transfer, NFT settlement)
        // Transfer assets back to owner
        let contract_address = e.current_contract_address();
        let token_client = token::Client::new(&e, &commitment.asset_address);
        token_client.transfer(&contract_address, &commitment.owner, &settlement_amount);

        // Call NFT contract to mark NFT as settled
        let nft_contract = e
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::NftContract)
            .unwrap_or_else(|| {
                set_reentrancy_guard(&e, false);
                panic!("NFT contract not initialized")
            });
        
        let mut args = Vec::new(&e);
        args.push_back(commitment.nft_token_id.into_val(&e));
        e.invoke_contract::<()>(&nft_contract, &Symbol::new(&e, "settle"), args);

        // Clear reentrancy guard
        set_reentrancy_guard(&e, false);

        // Emit settlement event
        e.events().publish(
            (symbol_short!("Settled"), commitment_id),
            (settlement_amount, e.ledger().timestamp()),
        );
    }

    /// Early exit (with penalty)
    /// 
    /// # Reentrancy Protection
    /// Uses checks-effects-interactions pattern with reentrancy guard.
    pub fn early_exit(e: Env, commitment_id: String, caller: Address) {
        // Reentrancy protection
        require_no_reentrancy(&e);
        set_reentrancy_guard(&e, true);

        // CHECKS: Get and validate commitment
        let mut commitment = read_commitment(&e, &commitment_id)
            .unwrap_or_else(|| {
                set_reentrancy_guard(&e, false);
                panic!("Commitment not found")
            });

        // Verify caller is owner
        if commitment.owner != caller {
            set_reentrancy_guard(&e, false);
            panic!("Unauthorized: caller is not the owner");
        }

        // Verify commitment is active
        let active_status = String::from_str(&e, "active");
        if commitment.status != active_status {
            set_reentrancy_guard(&e, false);
            panic!("Commitment is not active");
        }

        // EFFECTS: Calculate penalty using shared utilities
        let penalty_amount =
            SafeMath::penalty_amount(commitment.current_value, commitment.rules.early_exit_penalty);
        let returned_amount = SafeMath::sub(commitment.current_value, penalty_amount);

        commitment.status = String::from_str(&e, "early_exit");
        set_commitment(&e, &commitment);

        // Decrease total value locked by full current value (no longer locked)
        let current_tvl = e
            .storage()
            .instance()
            .get::<_, i128>(&DataKey::TotalValueLocked)
            .unwrap_or(0);
        let new_tvl = current_tvl - commitment.current_value;
        e.storage()
            .instance()
            .set(&DataKey::TotalValueLocked, &new_tvl);

        // INTERACTIONS: External calls (token transfer)
        // Transfer remaining amount (after penalty) to owner
        let contract_address = e.current_contract_address();
        let token_client = token::Client::new(&e, &commitment.asset_address);
        token_client.transfer(&contract_address, &commitment.owner, &returned_amount);

        // Clear reentrancy guard
        set_reentrancy_guard(&e, false);

        // Emit early exit event
        e.events().publish(
            (symbol_short!("EarlyExt"), commitment_id, caller),
            (penalty_amount, returned_amount, e.ledger().timestamp()),
        );
    }

    /// Allocate liquidity (called by allocation strategy)
    /// 
    /// # Reentrancy Protection
    /// Uses checks-effects-interactions pattern with reentrancy guard.
    pub fn allocate(e: Env, commitment_id: String, target_pool: Address, amount: i128) {
        // Reentrancy protection
        require_no_reentrancy(&e);
        set_reentrancy_guard(&e, true);

        // Rate limit allocations per target pool address
        let fn_symbol = symbol_short!("alloc");
        RateLimiter::check(&e, &target_pool, &fn_symbol);

        // CHECKS: Validate inputs and commitment
        if amount <= 0 {
            set_reentrancy_guard(&e, false);
            panic!("Invalid amount");
        }

        let commitment = read_commitment(&e, &commitment_id)
            .unwrap_or_else(|| {
                set_reentrancy_guard(&e, false);
                panic!("Commitment not found")
            });

        // Verify commitment is active
        let active_status = String::from_str(&e, "active");
        if commitment.status != active_status {
            set_reentrancy_guard(&e, false);
            panic!("Commitment is not active");
        }

        // Verify sufficient balance
        if commitment.current_value < amount {
            set_reentrancy_guard(&e, false);
            panic!("Insufficient commitment value");
        }

        // EFFECTS: Update commitment value before external call
        let mut updated_commitment = commitment;
        updated_commitment.current_value = updated_commitment.current_value - amount;
        set_commitment(&e, &updated_commitment);

        // INTERACTIONS: External call (token transfer)
        // Transfer assets to target pool
        let contract_address = e.current_contract_address();
        let token_client = token::Client::new(&e, &updated_commitment.asset_address);
        token_client.transfer(&contract_address, &target_pool, &amount);

        // Clear reentrancy guard
        set_reentrancy_guard(&e, false);

        // Emit allocation event
        e.events().publish(
            (symbol_short!("Alloc"), commitment_id, target_pool),
            (amount, e.ledger().timestamp()),
        );
    }

    /// Configure rate limits for this contract's functions.
    ///
    /// This function is restricted to the contract admin.
    pub fn set_rate_limit(
        e: Env,
        caller: Address,
        function: Symbol,
        window_seconds: u64,
        max_calls: u32,
    ) {
        require_admin(&e, &caller);
        RateLimiter::set_limit(&e, &function, window_seconds, max_calls);
    }

    /// Set or clear rate limit exemption for an address.
    ///
    /// This function is restricted to the contract admin.
    pub fn set_rate_limit_exempt(e: Env, caller: Address, address: Address, exempt: bool) {
        require_admin(&e, &caller);
        RateLimiter::set_exempt(&e, &address, exempt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{symbol_short, testutils::Address as _, Address, Env, String, Symbol, Vec};

    /* -------------------- DUMMY CONTRACTS -------------------- */

    #[contract]
    struct DummyTokenContract;

    #[contractimpl]
    impl DummyTokenContract {
        pub fn transfer(from: Address, to: Address, amount: i128) {
            // record transfer for assertions
        }
    }

    #[contract]
    struct DummyNFTContract;

    #[contractimpl]
    impl DummyNFTContract {
        pub fn mint(owner: Address, commitment_id: String) -> u32 {
            1
        }

        pub fn mark_settled(token_id: u32) {
            // record settled
        }
    }

    /* -------------------- HELPER FUNCTIONS -------------------- */

    fn create_test_commitment(e: &Env, id: &str, owner: Address, expired: bool) -> Commitment {
        let now = e.ledger().timestamp();
        let (created_at, expires_at) = if expired {
            (now - 10000, now - 100)
        } else {
            (now, now + 10000)
        };

        Commitment {
            commitment_id: String::from_str(e, id),
            owner,
            nft_token_id: 1,
            rules: CommitmentRules {
                duration_days: 7,
                max_loss_percent: 20,
                commitment_type: String::from_str(e, "balanced"),
                early_exit_penalty: 5,
                min_fee_threshold: 0,
                grace_period_days: 3,
            },
            amount: 1000,
            asset_address: Address::generate(e),
            created_at,
            expires_at,
            current_value: 1000,
            status: String::from_str(e, "active"),
        }
    }

    fn setup_test_env() -> (Env, Address, Address, Address) {
        let e = Env::default();
        let token_id = e.register_contract(None, DummyTokenContract);
        let nft_id = e.register_contract(None, DummyNFTContract);
        let core_id = e.register_contract(None, CommitmentCoreContract);

        (e, token_id, nft_id, core_id)
    }

    /* -------------------- TESTS -------------------- */

    #[test]
    fn test_initialize() {
        let e = Env::default();
        let admin = Address::generate(&e);
        let nft_contract = Address::generate(&e);
        let contract_id = e.register_contract(None, CommitmentCoreContract);
        
        CommitmentCoreContract::initialize(e.clone(), admin.clone(), nft_contract.clone());
        
        let stored_admin: Address = e.storage().instance().get(&Symbol::short("ADMIN")).unwrap();
        let stored_nft: Address = e.storage().instance().get(&Symbol::short("NFT")).unwrap();
        
        assert_eq!(stored_admin, admin);
        assert_eq!(stored_nft, nft_contract);
    }

    #[test]
    fn test_settlement_flow_basic() {
        let (e, token_addr, nft_addr, core_addr) = setup_test_env();
        
        let owner = Address::generate(&e);
        let admin = Address::generate(&e);
        
        // Initialize contract
        CommitmentCoreContract::initialize(e.clone(), admin.clone(), nft_addr.clone());
        
        // Create an expired commitment
        let now = e.ledger().timestamp();
        let commitment = Commitment {
            commitment_id: String::from_str(&e, "settle_test_1"),
            owner: owner.clone(),
            nft_token_id: 101,
            rules: CommitmentRules {
                duration_days: 1,
                max_loss_percent: 10,
                commitment_type: String::from_str(&e, "safe"),
                early_exit_penalty: 5,
                min_fee_threshold: 0,
                grace_period_days: 2,
            },
            amount: 5000,
            asset_address: token_addr.clone(),
            created_at: now - 100000,
            expires_at: now - 1000,
            current_value: 5500,
            status: String::from_str(&e, "active"),
        };
        
        let mut commitments: Vec<Commitment> = Vec::new(&e);
        commitments.push_back(commitment.clone());
        e.storage().instance().set(&Symbol::short("COMMS"), &commitments);
        
        // Settle the commitment
        CommitmentCoreContract::settle(e.clone(), String::from_str(&e, "settle_test_1"));
        
        // Verify settlement (commitment removed from active list)
        let updated_commitments: Vec<Commitment> = e.storage().instance().get(&Symbol::short("COMMS")).unwrap();
        assert_eq!(updated_commitments.len(), 0); // Commitment should be removed
    }

    #[test]
    #[should_panic(expected = "Commitment not expired and grace period has passed")]
    fn test_settlement_rejects_active_commitment() {
        let (e, token_addr, nft_addr, _core_addr) = setup_test_env();
        
        let owner = Address::generate(&e);
        let admin = Address::generate(&e);
        
        // Initialize
        CommitmentCoreContract::initialize(e.clone(), admin.clone(), nft_addr.clone());
        
        // Create non-expired commitment
        let commitment = create_test_commitment(&e, "not_expired", owner.clone(), false);
        
        let mut commitments: Vec<Commitment> = Vec::new(&e);
        commitments.push_back(commitment);
        e.storage().instance().set(&Symbol::short("COMMS"), &commitments);
        
        // Try to settle; should panic
        CommitmentCoreContract::settle(e.clone(), String::from_str(&e, "not_expired"));
    }

    #[test]
    #[should_panic(expected = "Commitment not found")]
    fn test_settlement_commitment_not_found() {
        let (e, _token_addr, nft_addr, _core_addr) = setup_test_env();
        
        let admin = Address::generate(&e);
        
        // Initialize
        CommitmentCoreContract::initialize(e.clone(), admin.clone(), nft_addr.clone());
        
        // Try to settle non-existent commitment
        CommitmentCoreContract::settle(e.clone(), String::from_str(&e, "nonexistent"));
    }

    #[test]
    #[should_panic(expected = "Already settled")]
    fn test_settlement_already_settled() {
        let (e, token_addr, nft_addr, _core_addr) = setup_test_env();
        
        let owner = Address::generate(&e);
        let admin = Address::generate(&e);
        
        // Initialize
        CommitmentCoreContract::initialize(e.clone(), admin.clone(), nft_addr.clone());
        
        // Create expired commitment already settled
        let now = e.ledger().timestamp();
        let mut commitment = create_test_commitment(&e, "already_settled", owner.clone(), true);
        commitment.status = String::from_str(&e, "settled");
        
        let mut commitments: Vec<Commitment> = Vec::new(&e);
        commitments.push_back(commitment);
        e.storage().instance().set(&Symbol::short("COMMS"), &commitments);
        
        // Try to settle already settled commitment; should panic
        CommitmentCoreContract::settle(e.clone(), String::from_str(&e, "already_settled"));
    }

    #[test]
    fn test_expiration_check_expired() {
        let (e, _token_addr, nft_addr, _core_addr) = setup_test_env();
        
        let admin = Address::generate(&e);
        let owner = Address::generate(&e);
        
        // Initialize
        CommitmentCoreContract::initialize(e.clone(), admin.clone(), nft_addr.clone());
        
        // Create expired commitment
        let commitment = create_test_commitment(&e, "expired_check", owner, true);
        let mut commitments: Vec<Commitment> = Vec::new(&e);
        commitments.push_back(commitment);
        e.storage().instance().set(&Symbol::short("COMMS"), &commitments);
        
        // Check violations
        let is_violated = CommitmentCoreContract::check_violations(
            e.clone(),
            String::from_str(&e, "expired_check"),
        );
        assert!(is_violated);
    }

    #[test]
    fn test_expiration_check_not_expired() {
        let (e, _token_addr, nft_addr, _core_addr) = setup_test_env();
        
        let admin = Address::generate(&e);
        let owner = Address::generate(&e);
        
        // Initialize
        CommitmentCoreContract::initialize(e.clone(), admin.clone(), nft_addr.clone());
        
        // Create active (non-expired) commitment
        let commitment = create_test_commitment(&e, "not_expired_check", owner, false);
        let mut commitments: Vec<Commitment> = Vec::new(&e);
        commitments.push_back(commitment);
        e.storage().instance().set(&Symbol::short("COMMS"), &commitments);
        
        // Check violations
        let is_violated = CommitmentCoreContract::check_violations(
            e.clone(),
            String::from_str(&e, "not_expired_check"),
        );
        assert!(!is_violated);
    }

    #[test]
    fn test_asset_transfer_on_settlement() {
        let (e, token_addr, nft_addr, _core_addr) = setup_test_env();
        
        let owner = Address::generate(&e);
        let admin = Address::generate(&e);
        let settlement_amount = 7500i128;
        
        // Initialize
        CommitmentCoreContract::initialize(e.clone(), admin.clone(), nft_addr.clone());
        
        // Create expired commitment
        let now = e.ledger().timestamp();
        let mut commitment = Commitment {
            commitment_id: String::from_str(&e, "transfer_test"),
            owner: owner.clone(),
            nft_token_id: 102,
            rules: CommitmentRules {
                duration_days: 5,
                max_loss_percent: 15,
                commitment_type: String::from_str(&e, "growth"),
                early_exit_penalty: 10,
                min_fee_threshold: 0,
                grace_period_days: 1,
            },
            amount: 5000,
            asset_address: token_addr.clone(),
            created_at: now - 500000,
            expires_at: now - 10000,
            current_value: settlement_amount,
            status: String::from_str(&e, "active"),
        };
        
        let mut commitments: Vec<Commitment> = Vec::new(&e);
        commitments.push_back(commitment);
        e.storage().instance().set(&Symbol::short("COMMS"), &commitments);
        
        // Settle - this will call token transfer
        CommitmentCoreContract::settle(e.clone(), String::from_str(&e, "transfer_test"));
        
        // Verify the commitment is removed from active list (settled)
        let updated_commitments: Vec<Commitment> = e.storage().instance().get(&Symbol::short("COMMS")).unwrap();
        assert_eq!(updated_commitments.len(), 0); // Commitment should be removed after settlement
    }

    #[test]
    fn test_settlement_with_different_values() {
        let (e, _token_addr, nft_addr, _core_addr) = setup_test_env();
        
        let owner = Address::generate(&e);
        let admin = Address::generate(&e);
        
        // Initialize
        CommitmentCoreContract::initialize(e.clone(), admin.clone(), nft_addr.clone());
        
        let now = e.ledger().timestamp();
        
        // Test case 1: Settlement with gain
        let commitment_gain = Commitment {
            commitment_id: String::from_str(&e, "gain_test"),
            owner: owner.clone(),
            nft_token_id: 201,
            rules: CommitmentRules {
                duration_days: 30,
                max_loss_percent: 5,
                commitment_type: String::from_str(&e, "stable"),
                early_exit_penalty: 2,
                min_fee_threshold: 0,
                grace_period_days: 7,
            },
            amount: 10000,
            asset_address: Address::generate(&e),
            created_at: now - 2592000,
            expires_at: now - 1,
            current_value: 11000,
            status: String::from_str(&e, "active"),
        };
        
        let mut commitments: Vec<Commitment> = Vec::new(&e);
        commitments.push_back(commitment_gain);
        e.storage().instance().set(&Symbol::short("COMMS"), &commitments);
        
        CommitmentCoreContract::settle(e.clone(), String::from_str(&e, "gain_test"));
        
        let updated: Vec<Commitment> = e.storage().instance().get(&Symbol::short("COMMS")).unwrap();
        assert_eq!(updated.len(), 0); // Commitment should be removed after settlement
    }

    #[test]
    fn test_cross_contract_nft_settlement() {
        let (e, token_addr, nft_addr, _core_addr) = setup_test_env();
        
        let owner = Address::generate(&e);
        let admin = Address::generate(&e);
        let nft_token_id = 999u32;
        
        // Initialize
        CommitmentCoreContract::initialize(e.clone(), admin.clone(), nft_addr.clone());
        
        // Create expired commitment with specific NFT ID
        let now = e.ledger().timestamp();
        let commitment = Commitment {
            commitment_id: String::from_str(&e, "nft_cross_contract"),
            owner: owner.clone(),
            nft_token_id,
            rules: CommitmentRules {
                duration_days: 1,
                max_loss_percent: 10,
                commitment_type: String::from_str(&e, "safe"),
                early_exit_penalty: 5,
                min_fee_threshold: 0,
                grace_period_days: 1,
            },
            amount: 2000,
            asset_address: token_addr.clone(),
            created_at: now - 100000,
            expires_at: now - 1000,
            current_value: 2000,
            status: String::from_str(&e, "active"),
        };
        
        let mut commitments: Vec<Commitment> = Vec::new(&e);
        commitments.push_back(commitment);
        e.storage().instance().set(&Symbol::short("COMMS"), &commitments);
        
        // Settle - this will invoke NFT contract
        CommitmentCoreContract::settle(e.clone(), String::from_str(&e, "nft_cross_contract"));
        
        // Verify settlement completed (commitment removed from active list)
        let updated_commitments: Vec<Commitment> = e.storage().instance().get(&Symbol::short("COMMS")).unwrap();
        assert_eq!(updated_commitments.len(), 0); // Commitment should be removed after settlement
    }

    #[test]
    fn test_settlement_removes_commitment_status() {
        let (e, _token_addr, nft_addr, _core_addr) = setup_test_env();
        
        let owner = Address::generate(&e);
        let admin = Address::generate(&e);
        
        // Initialize
        CommitmentCoreContract::initialize(e.clone(), admin.clone(), nft_addr.clone());
        
        // Create multiple commitments
        let now = e.ledger().timestamp();
        let commitment1 = Commitment {
            commitment_id: String::from_str(&e, "multi_1"),
            owner: owner.clone(),
            nft_token_id: 301,
            rules: CommitmentRules {
                duration_days: 1,
                max_loss_percent: 10,
                commitment_type: String::from_str(&e, "safe"),
                early_exit_penalty: 5,
                min_fee_threshold: 0,
                grace_period_days: 1,
            },
            amount: 1000,
            asset_address: Address::generate(&e),
            created_at: now - 100000,
            expires_at: now - 1000,
            current_value: 1000,
            status: String::from_str(&e, "active"),
        };
        
        let commitment2 = Commitment {
            commitment_id: String::from_str(&e, "multi_2"),
            owner: owner.clone(),
            nft_token_id: 302,
            rules: CommitmentRules {
                duration_days: 30,
                max_loss_percent: 20,
                commitment_type: String::from_str(&e, "growth"),
                early_exit_penalty: 10,
                min_fee_threshold: 0,
                grace_period_days: 5,
            },
            amount: 2000,
            asset_address: Address::generate(&e),
            created_at: now,
            expires_at: now + 2592000,
            current_value: 2000,
            status: String::from_str(&e, "active"),
        };
        
        let mut commitments: Vec<Commitment> = Vec::new(&e);
        commitments.push_back(commitment1);
        commitments.push_back(commitment2);
        e.storage().instance().set(&Symbol::short("COMMS"), &commitments);
        
        // Settle first commitment
        CommitmentCoreContract::settle(e.clone(), String::from_str(&e, "multi_1"));
        
        // Verify only first is removed (settled commitments are removed from active list)
        let updated_commitments: Vec<Commitment> = e.storage().instance().get(&Symbol::short("COMMS")).unwrap();
        assert_eq!(updated_commitments.len(), 1); // Only commitment2 should remain
        assert_eq!(updated_commitments.get(0).unwrap().commitment_id, String::from_str(&e, "multi_2"));
        assert_eq!(updated_commitments.get(0).unwrap().status, String::from_str(&e, "active"));
    }
}
mod tests;
mod tests;
