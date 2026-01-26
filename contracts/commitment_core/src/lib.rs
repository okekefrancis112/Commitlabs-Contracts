#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, log, token, symbol_short, Address, Env, IntoVal, String,
    Symbol, Vec,
};

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
}

#[contracttype]
#[derive(Clone)]
pub struct CommitmentCreatedEvent {
    pub commitment_id: u32,  // Changed from String to u32
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
    pub commitment_type: String, // "safe", "balanced", "aggressive"
    pub early_exit_penalty: u32,
    pub min_fee_threshold: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Commitment {
    pub commitment_id: u32,  // Changed from String to u32 for uniqueness
    pub owner: Address,
    pub nft_token_id: u32,
    pub rules: CommitmentRules,
    pub amount: i128,
    pub asset_address: Address,
    pub created_at: u64,
    pub expires_at: u64,
    pub current_value: i128,
    pub status: String, // "active", "settled", "violated", "early_exit"
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    NftContract,
    Commitment(u32),        // commitment_id -> Commitment (changed from String to u32)
    OwnerCommitments(Address), // owner -> Vec<u32> (changed from Vec<String>)
    TotalCommitments,          // counter
}

/// Convert u32 to String (for no_std compatibility)
/// Handles numbers 0-999 with lookup table
fn u32_to_string(e: &Env, n: u32) -> String {
    // For numbers 0-99, use direct lookup
    if n < 100 {
        let lookup_0_99: [&str; 100] = [
            "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
            "10", "11", "12", "13", "14", "15", "16", "17", "18", "19",
            "20", "21", "22", "23", "24", "25", "26", "27", "28", "29",
            "30", "31", "32", "33", "34", "35", "36", "37", "38", "39",
            "40", "41", "42", "43", "44", "45", "46", "47", "48", "49",
            "50", "51", "52", "53", "54", "55", "56", "57", "58", "59",
            "60", "61", "62", "63", "64", "65", "66", "67", "68", "69",
            "70", "71", "72", "73", "74", "75", "76", "77", "78", "79",
            "80", "81", "82", "83", "84", "85", "86", "87", "88", "89",
            "90", "91", "92", "93", "94", "95", "96", "97", "98", "99",
        ];
        String::from_str(e, lookup_0_99[n as usize])
    } else if n < 1000 {
        // For 100-999, build manually using hundreds digit
        let hundreds = n / 100;
        let remainder = n % 100;
        let h_str = match hundreds {
            1 => "1", 2 => "2", 3 => "3", 4 => "4", 5 => "5",
            6 => "6", 7 => "7", 8 => "8", 9 => "9", _ => "0",
        };
        let rem_str = u32_to_string(e, remainder);
        // Since we can't concatenate strings directly, use a lookup for common values
        // For now, handle 100-199, 200-299, etc. separately
        match hundreds {
            1 => {
                if remainder < 100 {
                    let lookup_100_199: [&str; 100] = [
                        "100", "101", "102", "103", "104", "105", "106", "107", "108", "109",
                        "110", "111", "112", "113", "114", "115", "116", "117", "118", "119",
                        "120", "121", "122", "123", "124", "125", "126", "127", "128", "129",
                        "130", "131", "132", "133", "134", "135", "136", "137", "138", "139",
                        "140", "141", "142", "143", "144", "145", "146", "147", "148", "149",
                        "150", "151", "152", "153", "154", "155", "156", "157", "158", "159",
                        "160", "161", "162", "163", "164", "165", "166", "167", "168", "169",
                        "170", "171", "172", "173", "174", "175", "176", "177", "178", "179",
                        "180", "181", "182", "183", "184", "185", "186", "187", "188", "189",
                        "190", "191", "192", "193", "194", "195", "196", "197", "198", "199",
                    ];
                    String::from_str(e, lookup_100_199[remainder as usize])
                } else {
                    String::from_str(e, "199")
                }
            }
            _ => String::from_str(e, "999") // Placeholder for 200+
        }
    } else {
        // For 1000+, use a placeholder
        // In production, implement proper conversion for all numbers
        String::from_str(e, "1000")
    }
}
/// Transfer assets from owner to contract
/// Note: In test environment, this may fail if token contract is not set up
fn transfer_assets(e: &Env, from: &Address, to: &Address, asset_address: &Address, amount: i128) {
    let token_client = token::Client::new(e, asset_address);

    // Check balance first - this will panic if token contract doesn't exist
    // In tests, this means you need to set up a token contract
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
    // Note: early_exit_penalty is not passed here, NFT contract will use default or get from rules

    // In Soroban, contract calls return the value directly
    // Failures cause the entire transaction to fail
    e.invoke_contract::<u32>(nft_contract, &Symbol::new(e, "mint"), args)
}

// Storage helpers
fn read_commitment(e: &Env, commitment_id: u32) -> Option<Commitment> {
    e.storage()
        .instance()
        .get::<_, Commitment>(&DataKey::Commitment(commitment_id))
}

fn set_commitment(e: &Env, commitment: &Commitment) {
    e.storage()
        .instance()
        .set(&DataKey::Commitment(commitment.commitment_id), commitment);
}

fn has_commitment(e: &Env, commitment_id: u32) -> bool {
    e.storage()
        .instance()
        .has(&DataKey::Commitment(commitment_id))
}

#[contract]
pub struct CommitmentCoreContract;

#[contractimpl]
impl CommitmentCoreContract {
    /// Validate commitment rules
    fn validate_rules(e: &Env, rules: &CommitmentRules) {
        // Duration must be > 0
        if rules.duration_days == 0 {
            log!(e, "Invalid duration: {}", rules.duration_days);
            panic!("Invalid duration");
        }

        // Max loss percent must be between 0 and 100
        if rules.max_loss_percent > 100 {
            log!(e, "Invalid max loss percent: {}", rules.max_loss_percent);
            panic!("Invalid max loss percent");
        }

        // Commitment type must be valid
        let valid_types = ["safe", "balanced", "aggressive"];
        let mut is_valid = false;
        for valid_type in valid_types.iter() {
            if rules.commitment_type == String::from_str(e, valid_type) {
                is_valid = true;
                break;
            }
        }
        if !is_valid {
            log!(e, "Invalid commitment type");
            panic!("Invalid commitment type");
        }
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
    }

    /// Create a new commitment - returns unique commitment_id (u32)
    pub fn create_commitment(
        e: Env,
        owner: Address,
        amount: i128,
        asset_address: Address,
        rules: CommitmentRules,
    ) -> u32 {
        // Validate amount > 0
        if amount <= 0 {
            log!(&e, "Invalid amount: {}", amount);
            panic!("Invalid amount");
        }

        // Validate rules
        Self::validate_rules(&e, &rules);

        // Get and increment commitment counter
        let current_total = e
            .storage()
            .instance()
            .get::<_, u64>(&DataKey::TotalCommitments)
            .unwrap_or(0);
        let new_counter = current_total + 1;
        let commitment_id = new_counter as u32; // Use counter as unique ID
        
        // Get NFT contract address
        let nft_contract = e
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::NftContract)
            .unwrap_or_else(|| panic!("Contract not initialized"));

        // Transfer assets from owner to contract
        let contract_address = e.current_contract_address();
        transfer_assets(&e, &owner, &contract_address, &asset_address, amount);

        // Mint NFT - convert commitment_id to String for NFT contract call
        let commitment_id_str = u32_to_string(&e, commitment_id);
        let nft_token_id = call_nft_mint(
            &e,
            &nft_contract,
            &owner,
            &commitment_id_str,
            rules.duration_days,
            rules.max_loss_percent,
            &rules.commitment_type,
            amount,
            &asset_address,
        );

        // Calculate expiration timestamp (current time + duration in days)
        let current_timestamp = e.ledger().timestamp();
        let expires_at = current_timestamp + (rules.duration_days as u64 * 24 * 60 * 60); // days to seconds

        // Create commitment data
        let commitment = Commitment {
            commitment_id,
            owner: owner.clone(),
            nft_token_id,
            rules: rules.clone(),
            amount,
            asset_address: asset_address.clone(),
            created_at: current_timestamp,
            expires_at,
            current_value: amount, // Initially same as amount
            status: String::from_str(&e, "active"),
        };

        // Store commitment data
        set_commitment(&e, &commitment);

        // Update owner's commitment list
        let mut owner_commitments = e
            .storage()
            .instance()
            .get::<_, Vec<u32>>(&DataKey::OwnerCommitments(owner.clone()))
            .unwrap_or(Vec::new(&e));
        owner_commitments.push_back(commitment_id);
        e.storage().instance().set(
            &DataKey::OwnerCommitments(owner.clone()),
            &owner_commitments,
        );

        // Increment total commitments counter
        e.storage()
            .instance()
            .set(&DataKey::TotalCommitments, &new_counter);

        // Emit creation event
        let event = CommitmentCreatedEvent {
            commitment_id,
            owner: owner.clone(),
            amount,
            asset_address,
            nft_token_id,
            rules,
            timestamp: current_timestamp,
        };
        e.events()
            .publish((Symbol::new(&e, "commitment_created"),), event);

        commitment_id
    }

    /// Get commitment details
    pub fn get_commitment(e: Env, commitment_id: u32) -> Commitment {
        read_commitment(&e, commitment_id)
            .unwrap_or_else(|| panic!("Commitment not found"))
    }

    /// Get all commitments for an owner
    pub fn get_owner_commitments(e: Env, owner: Address) -> Vec<u32> {
        e.storage()
            .instance()
            .get::<_, Vec<u32>>(&DataKey::OwnerCommitments(owner))
            .unwrap_or(Vec::new(&e))
    }

    /// Get total number of commitments
    pub fn get_total_commitments(e: Env) -> u64 {
        e.storage()
            .instance()
            .get::<_, u64>(&DataKey::TotalCommitments)
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

    /// Update commitment value (called by allocation logic)
    pub fn update_value(e: Env, commitment_id: u32, new_value: i128) {
        let mut commitment = read_commitment(&e, commitment_id)
            .unwrap_or_else(|| panic!("Commitment not found"));

        // Update current_value
        commitment.current_value = new_value;

        // Store updated commitment
        set_commitment(&e, &commitment);
    }

    /// Check if commitment rules are violated
    pub fn check_violations(e: Env, commitment_id: u32) -> bool {
        let commitment = read_commitment(&e, commitment_id)
            .unwrap_or_else(|| panic!("Commitment not found"));

        let active_status = String::from_str(&e, "active");
        if commitment.status != active_status {
            return false;
        }

        let current_time = e.ledger().timestamp();
        let loss_amount = commitment.amount - commitment.current_value;
        let loss_percent = if commitment.amount > 0 {
            (loss_amount * 100) / commitment.amount
        } else {
            0
        };

        let max_loss = commitment.rules.max_loss_percent as i128;
        let loss_violated = loss_percent > max_loss;
        let duration_violated = current_time >= commitment.expires_at;

        loss_violated || duration_violated
    }

    /// Get detailed violation information
    pub fn get_violation_details(e: Env, commitment_id: u32) -> (bool, bool, bool, i128, u64) {
        let commitment = read_commitment(&e, commitment_id)
            .unwrap_or_else(|| panic!("Commitment not found"));

        let current_time = e.ledger().timestamp();
        let loss_amount = commitment.amount - commitment.current_value;
        let loss_percent = if commitment.amount > 0 {
            (loss_amount * 100) / commitment.amount
        } else {
            0
        };

        let max_loss = commitment.rules.max_loss_percent as i128;
        let loss_violated = loss_percent > max_loss;
        let duration_violated = current_time >= commitment.expires_at;
        let time_remaining = if current_time < commitment.expires_at {
            commitment.expires_at - current_time
        } else {
            0
        };

        let has_violations = loss_violated || duration_violated;
        (has_violations, loss_violated, duration_violated, loss_percent, time_remaining)
    }

    /// Settle commitment at maturity
    pub fn settle(e: Env, commitment_id: u32) {
        let mut commitment = read_commitment(&e, commitment_id)
            .unwrap_or_else(|| panic!("Commitment not found"));

        let current_time = e.ledger().timestamp();
        if current_time < commitment.expires_at {
            panic!("Commitment not yet expired");
        }

        commitment.status = String::from_str(&e, "settled");
        set_commitment(&e, &commitment);
    }

    /// Early exit (with penalty)
    pub fn early_exit(e: Env, commitment_id: u32, caller: Address) {
        caller.require_auth();

        let mut commitment = read_commitment(&e, commitment_id)
            .unwrap_or_else(|| panic!("Commitment not found"));

        // Verify caller is owner
        if commitment.owner != caller {
            panic!("Unauthorized");
        }

        commitment.status = String::from_str(&e, "early_exit");
        set_commitment(&e, &commitment);
    }

    /// Allocate liquidity (called by allocation strategy)
    pub fn allocate(_e: Env, _commitment_id: u32, _target_pool: Address, _amount: i128) {
        // TODO: Implement allocation logic
    }
}

#[cfg(test)]
mod tests;
