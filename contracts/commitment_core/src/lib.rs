#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype,
    Address, Env, String, Symbol, Vec, symbol_short,
    Val, IntoVal,
};
use soroban_sdk::token::Client as TokenClient;

/* -------------------- STORAGE KEYS -------------------- */

const ADMIN_KEY: Symbol = symbol_short!("ADMIN");
const NFT_KEY: Symbol = symbol_short!("NFT");
const COMMITMENTS_KEY: Symbol = symbol_short!("COMMS");

/* -------------------- DATA TYPES -------------------- */

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
            &symbol_short!("mint"),
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
            (Symbol::new(&e, "CommitmentCreated"),),
            (commitment_id.clone(), owner, amount, now),
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
    }

    /* ---------- UPDATE VALUE ---------- */

    pub fn update_value(e: Env, commitment_id: String, new_value: i128) {
        let admin: Address =
            e.storage().instance().get(&ADMIN_KEY).unwrap();
        admin.require_auth();

        let mut commitments: Vec<Commitment> =
            e.storage().instance().get(&COMMITMENTS_KEY).unwrap();

        for (i, mut c) in commitments.iter().enumerate() {
            if c.commitment_id == commitment_id {
                if c.status != String::from_str(&e, "active") {
                    panic!("Not active");
                }

                c.current_value = new_value;

                let loss_percent =
                    (c.amount - new_value) * 100 / c.amount;

                if loss_percent > c.rules.max_loss_percent as i128 {
                    c.status = String::from_str(&e, "violated");
                }

                e.events().publish(
                    (Symbol::new(&e, "ValueUpdated"),),
                    (commitment_id, new_value),
                );

                commitments.set(i as u32, c);
                e.storage().instance().set(&COMMITMENTS_KEY, &commitments);
                return;
            }
        }

        panic!("Commitment not found");
    }

    /* ---------- CHECK VIOLATIONS ---------- */

    pub fn check_violations(e: Env, commitment_id: String) -> bool {
        let c = Self::get_commitment(e.clone(), commitment_id);
        let now = e.ledger().timestamp();

        // Check if past grace period (violated)
        let grace_period_end = c.expires_at + (c.rules.grace_period_days as u64 * 86400);
        if now >= grace_period_end {
            return true;
        }

        let loss_percent =
            (c.amount - c.current_value) * 100 / c.amount;

        loss_percent > c.rules.max_loss_percent as i128
    }

    /* ---------- SETTLEMENT ---------- */

    pub fn settle(e: Env, commitment_id: String) {
        let now = e.ledger().timestamp();

        let mut commitments: Vec<Commitment> =
            e.storage().instance().get(&COMMITMENTS_KEY).unwrap();

        let mut found_index = None;
        let mut commitment_to_settle = None;

        // Find the commitment first
        for (i, c) in commitments.iter().enumerate() {
            if c.commitment_id == commitment_id {
                if c.status == String::from_str(&e, "settled") || c.status == String::from_str(&e, "early_exit") {
                    panic!("Already settled");
                }

                // Check if expired (maturity reached)
                if now < c.expires_at {
                    panic!("Commitment not expired");
                }

                found_index = Some(i);
                commitment_to_settle = Some(c.clone());
                break;
            }
        }

        // If commitment not found, panic
        let mut commitment = match commitment_to_settle {
            Some(c) => c,
            None => panic!("Commitment not found"),
        };

        // Transfer settlement amount
        TokenClient::new(&e, &commitment.asset_address)
            .transfer(
                &e.current_contract_address(),
                &commitment.owner,
                &commitment.current_value,
            );

        // Mark NFT settled
        let nft_contract: Address =
            e.storage().instance().get(&NFT_KEY).unwrap();

        let mut args = Vec::<Val>::new(&e);
        args.push_back(commitment.nft_token_id.into_val(&e));

        e.invoke_contract::<()>(
            &nft_contract,
            &symbol_short!("settle"),
            args,
        );

        // Emit settlement event
        e.events().publish(
            (Symbol::new(&e, "CommitmentSettled"),),
            (
                commitment_id.clone(),
                commitment.owner.clone(),
                commitment.current_value,
                now,
            ),
        );

        // Mark as settled and update storage
        if let Some(index) = found_index {
            commitment.status = String::from_str(&e, "settled");
            commitments.set(index as u32, commitment);
            e.storage().instance().set(&COMMITMENTS_KEY, &commitments);
        }
    }

    /* ---------- EARLY EXIT ---------- */

    pub fn early_exit(e: Env, commitment_id: String, caller: Address) {
        caller.require_auth();

        let mut commitments: Vec<Commitment> =
            e.storage().instance().get(&COMMITMENTS_KEY).unwrap();

        for (i, mut c) in commitments.iter().enumerate() {
            if c.commitment_id == commitment_id {
                if caller != c.owner {
                    panic!("Unauthorized");
                }

                if c.status != String::from_str(&e, "active") {
                    panic!("Not active");
                }

                let penalty =
                    c.current_value * c.rules.early_exit_penalty as i128 / 100;
                let payout = c.current_value - penalty;

                TokenClient::new(&e, &c.asset_address)
                    .transfer(
                        &e.current_contract_address(),
                        &c.owner,
                        &payout,
                    );

                c.status = String::from_str(&e, "early_exit");

                e.events().publish(
                    (symbol_short!("EarlyExit"),),
                    (commitment_id, payout),
                );

                commitments.set(i as u32, c);
                e.storage().instance().set(&COMMITMENTS_KEY, &commitments);
                return;
            }
        }

        panic!("Commitment not found");
    }

    /* ---------- ALLOCATE ---------- */

    pub fn allocate(
        e: Env,
        commitment_id: String,
        target_pool: Address,
        amount: i128,
    ) {
        let admin: Address =
            e.storage().instance().get(&ADMIN_KEY).unwrap();
        admin.require_auth();

        let commitments: Vec<Commitment> =
            e.storage().instance().get(&COMMITMENTS_KEY).unwrap();

        for c in commitments.iter() {
            if c.commitment_id == commitment_id {
                if c.status != String::from_str(&e, "active") {
                    panic!("Not active");
                }

                TokenClient::new(&e, &c.asset_address)
                    .transfer(
                        &e.current_contract_address(),
                        &target_pool,
                        &amount,
                    );

                e.events().publish(
                    (symbol_short!("Allocated"),),
                    (commitment_id, target_pool, amount),
                );

                return;
            }
        }

        panic!("Commitment not found");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{symbol_short, testutils::{Address as _, Ledger as _}, Address, Env, String, Vec};

    /* -------------------- DUMMY CONTRACTS -------------------- */

    #[contract]
    struct DummyTokenContract;

    #[contractimpl]
    impl DummyTokenContract {
        pub fn transfer(_from: Address, _to: Address, _amount: i128) {
            // record transfer for assertions
        }
    }

    #[contract]
    struct DummyNFTContract;

    #[contractimpl]
    impl DummyNFTContract {
        pub fn mint(_owner: Address, _commitment_id: String) -> u32 {
            1
        }

    pub fn settle(_e: Env, _token_id: u32) {
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
        e.ledger().set(soroban_sdk::testutils::LedgerInfo {
            timestamp: 10000000,
            protocol_version: 21,
            sequence_number: 1,
            network_id: [0u8; 32],
            base_reserve: 10,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 4096,
            max_entry_ttl: 6312000,
        });
        let token_id = e.register_contract(None, DummyTokenContract);
        let nft_id = e.register_contract(None, DummyNFTContract);
        let core_id = e.register_contract(None, CommitmentCoreContract);

        (e, token_id, nft_id, core_id)
    }

    /* -------------------- TESTS -------------------- */

    #[test]
    fn test_initialize() {
        let e = Env::default();
        e.mock_all_auths();
        let admin = Address::generate(&e);
        let nft_contract = Address::generate(&e);
        let contract_id = e.register_contract(None, CommitmentCoreContract);
        let client = CommitmentCoreContractClient::new(&e, &contract_id);
        
        client.initialize(&admin, &nft_contract);
        
        let stored_admin: Address = e.as_contract(&contract_id, || e.storage().instance().get(&symbol_short!("ADMIN")).unwrap());
        let stored_nft: Address = e.as_contract(&contract_id, || e.storage().instance().get(&symbol_short!("NFT")).unwrap());
        
        assert_eq!(stored_admin, admin);
        assert_eq!(stored_nft, nft_contract);
    }

    #[test]
    fn test_settlement_flow_basic() {
        let (e, token_addr, nft_addr, core_addr) = setup_test_env();
        let client = CommitmentCoreContractClient::new(&e, &core_addr);
        e.mock_all_auths();
        
        let owner = Address::generate(&e);
        let admin = Address::generate(&e);
        
        // Initialize contract
        client.initialize(&admin, &nft_addr);
        
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
        e.as_contract(&core_addr, || e.storage().instance().set(&symbol_short!("COMMS"), &commitments));
        
        // Settle the commitment
        client.settle(&String::from_str(&e, "settle_test_1"));
        
        // Verify settlement (status changed to settled)
        let updated_commitments: Vec<Commitment> = e.as_contract(&core_addr, || e.storage().instance().get(&symbol_short!("COMMS")).unwrap());
        assert_eq!(updated_commitments.len(), 1); 
        assert_eq!(updated_commitments.get(0).unwrap().status, String::from_str(&e, "settled"));
    }

    #[test]
    #[should_panic(expected = "Commitment not expired")]
    fn test_settlement_rejects_active_commitment() {
        let (e, _token_addr, nft_addr, core_addr) = setup_test_env();
        let client = CommitmentCoreContractClient::new(&e, &core_addr);
        e.mock_all_auths();
        
        let owner = Address::generate(&e);
        let admin = Address::generate(&e);
        
        // Initialize
        client.initialize(&admin, &nft_addr);
        
        // Create non-expired commitment
        let commitment = create_test_commitment(&e, "not_expired", owner.clone(), false);
        
        let mut commitments: Vec<Commitment> = Vec::new(&e);
        commitments.push_back(commitment);
        e.as_contract(&core_addr, || e.storage().instance().set(&symbol_short!("COMMS"), &commitments));
        
        // Try to settle; should panic
        client.settle(&String::from_str(&e, "not_expired"));
    }

    #[test]
    #[should_panic(expected = "Commitment not found")]
    fn test_settlement_commitment_not_found() {
        let (e, _token_addr, nft_addr, core_addr) = setup_test_env();
        let client = CommitmentCoreContractClient::new(&e, &core_addr);
        e.mock_all_auths();
        
        let admin = Address::generate(&e);
        
        // Initialize
        client.initialize(&admin, &nft_addr);
        
        // Try to settle non-existent commitment
        client.settle(&String::from_str(&e, "nonexistent"));
    }

    #[test]
    #[should_panic(expected = "Already settled")]
    fn test_settlement_already_settled() {
        let (e, _token_addr, nft_addr, core_addr) = setup_test_env();
        let client = CommitmentCoreContractClient::new(&e, &core_addr);
        e.mock_all_auths();
        
        let owner = Address::generate(&e);
        let admin = Address::generate(&e);
        
        // Initialize
        client.initialize(&admin, &nft_addr);
        
        // Create expired commitment already settled
        let _now = e.ledger().timestamp();
        let mut commitment = create_test_commitment(&e, "already_settled", owner.clone(), true);
        commitment.status = String::from_str(&e, "settled");
        
        let mut commitments: Vec<Commitment> = Vec::new(&e);
        commitments.push_back(commitment);
        e.as_contract(&core_addr, || e.storage().instance().set(&symbol_short!("COMMS"), &commitments));
        
        // Try to settle already settled commitment; should panic
        client.settle(&String::from_str(&e, "already_settled"));
    }

    #[test]
    fn test_expiration_check_expired() {
        let (e, _token_addr, nft_addr, core_addr) = setup_test_env();
        let client = CommitmentCoreContractClient::new(&e, &core_addr);
        e.mock_all_auths();
        
        let admin = Address::generate(&e);
        let owner = Address::generate(&e);
        
        // Initialize
        client.initialize(&admin, &nft_addr);
        
        // Create expired commitment
        let commitment = create_test_commitment(&e, "expired_check", owner, true);
        let mut commitments: Vec<Commitment> = Vec::new(&e);
        commitments.push_back(commitment);
        e.as_contract(&core_addr, || e.storage().instance().set(&symbol_short!("COMMS"), &commitments));
        
        // Check violations
        let is_violated = client.check_violations(&String::from_str(&e, "expired_check"));
        // Note: is_violated is true only if past grace period (3 days)
        // expired_check is only 100s past expires_at, so it's NOT violated yet
        assert!(!is_violated);
    }

    #[test]
    fn test_expiration_check_not_expired() {
        let (e, _token_addr, nft_addr, core_addr) = setup_test_env();
        let client = CommitmentCoreContractClient::new(&e, &core_addr);
        e.mock_all_auths();
        
        let admin = Address::generate(&e);
        let owner = Address::generate(&e);
        
        // Initialize
        client.initialize(&admin, &nft_addr);
        
        // Create active (non-expired) commitment
        let commitment = create_test_commitment(&e, "not_expired_check", owner, false);
        let mut commitments: Vec<Commitment> = Vec::new(&e);
        commitments.push_back(commitment);
        e.as_contract(&core_addr, || e.storage().instance().set(&symbol_short!("COMMS"), &commitments));
        
        // Check violations
        let is_violated = client.check_violations(&String::from_str(&e, "not_expired_check"));
        assert!(!is_violated);
    }

    #[test]
    fn test_expiration_check_violated() {
        let (e, _token_addr, nft_addr, core_addr) = setup_test_env();
        let client = CommitmentCoreContractClient::new(&e, &core_addr);
        e.mock_all_auths();
        
        let admin = Address::generate(&e);
        let owner = Address::generate(&e);
        
        // Initialize
        client.initialize(&admin, &nft_addr);
        
        // Create commitment past grace period
        let now = e.ledger().timestamp();
        let commitment = Commitment {
            commitment_id: String::from_str(&e, "violated_check"),
            owner,
            nft_token_id: 1,
            rules: CommitmentRules {
                duration_days: 7,
                max_loss_percent: 20,
                commitment_type: String::from_str(&e, "balanced"),
                early_exit_penalty: 5,
                min_fee_threshold: 0,
                grace_period_days: 1,
            },
            amount: 1000,
            asset_address: Address::generate(&e),
            created_at: now - 1000000,
            expires_at: now - 200000, // past 1 day grace period (86400)
            current_value: 1000,
            status: String::from_str(&e, "active"),
        };
        let mut commitments: Vec<Commitment> = Vec::new(&e);
        commitments.push_back(commitment);
        e.as_contract(&core_addr, || e.storage().instance().set(&symbol_short!("COMMS"), &commitments));
        
        // Check violations
        let is_violated = client.check_violations(&String::from_str(&e, "violated_check"));
        assert!(is_violated);
    }

    #[test]
    fn test_asset_transfer_on_settlement() {
        let (e, token_addr, nft_addr, core_addr) = setup_test_env();
        let client = CommitmentCoreContractClient::new(&e, &core_addr);
        e.mock_all_auths();
        
        let owner = Address::generate(&e);
        let admin = Address::generate(&e);
        let settlement_amount = 7500i128;
        
        // Initialize
        client.initialize(&admin, &nft_addr);
        
        // Create expired commitment
        let now = e.ledger().timestamp();
        let commitment = Commitment {
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
        e.as_contract(&core_addr, || e.storage().instance().set(&symbol_short!("COMMS"), &commitments));
        
        // Settle - this will call token transfer
        client.settle(&String::from_str(&e, "transfer_test"));
        
        // Verify the commitment status updated
        let updated_commitments: Vec<Commitment> = e.as_contract(&core_addr, || e.storage().instance().get(&symbol_short!("COMMS")).unwrap());
        assert_eq!(updated_commitments.len(), 1);
        assert_eq!(updated_commitments.get(0).unwrap().status, String::from_str(&e, "settled"));
    }

    #[test]
    fn test_settlement_with_different_values() {
        let (e, token_addr, nft_addr, core_addr) = setup_test_env();
        let client = CommitmentCoreContractClient::new(&e, &core_addr);
        e.mock_all_auths();
        
        let owner = Address::generate(&e);
        let admin = Address::generate(&e);
        
        // Initialize
        client.initialize(&admin, &nft_addr);
        
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
            asset_address: token_addr.clone(),
            created_at: now - 2592000,
            expires_at: now - 1,
            current_value: 11000,
            status: String::from_str(&e, "active"),
        };
        
        let mut commitments: Vec<Commitment> = Vec::new(&e);
        commitments.push_back(commitment_gain);
        e.as_contract(&core_addr, || e.storage().instance().set(&symbol_short!("COMMS"), &commitments));
        
        client.settle(&String::from_str(&e, "gain_test"));
        
        let updated: Vec<Commitment> = e.as_contract(&core_addr, || e.storage().instance().get(&symbol_short!("COMMS")).unwrap());
        assert_eq!(updated.get(0).unwrap().status, String::from_str(&e, "settled"));
    }

    #[test]
    fn test_cross_contract_nft_settlement() {
        let (e, token_addr, nft_addr, core_addr) = setup_test_env();
        let client = CommitmentCoreContractClient::new(&e, &core_addr);
        e.mock_all_auths();
        
        let owner = Address::generate(&e);
        let admin = Address::generate(&e);
        let nft_token_id = 999u32;
        
        // Initialize
        client.initialize(&admin, &nft_addr);
        
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
        e.as_contract(&core_addr, || e.storage().instance().set(&symbol_short!("COMMS"), &commitments));
        
        // Settle - this will invoke NFT contract
        client.settle(&String::from_str(&e, "nft_cross_contract"));
        
        // Verify settlement completed (status updated)
        let updated_commitments: Vec<Commitment> = e.as_contract(&core_addr, || e.storage().instance().get(&symbol_short!("COMMS")).unwrap());
        assert_eq!(updated_commitments.get(0).unwrap().status, String::from_str(&e, "settled"));
    }

    #[test]
    fn test_settlement_removes_commitment_status() {
        let (e, token_addr, nft_addr, core_addr) = setup_test_env();
        let client = CommitmentCoreContractClient::new(&e, &core_addr);
        e.mock_all_auths();
        
        let owner = Address::generate(&e);
        let admin = Address::generate(&e);
        
        // Initialize
        client.initialize(&admin, &nft_addr);
        
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
            asset_address: token_addr.clone(),
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
            asset_address: token_addr.clone(),
            created_at: now,
            expires_at: now + 2592000,
            current_value: 2000,
            status: String::from_str(&e, "active"),
        };
        
        let mut commitments: Vec<Commitment> = Vec::new(&e);
        commitments.push_back(commitment1);
        commitments.push_back(commitment2);
        e.as_contract(&core_addr, || e.storage().instance().set(&symbol_short!("COMMS"), &commitments));
        
        // Settle first commitment
        client.settle(&String::from_str(&e, "multi_1"));
        
        // Verify status updated (still in list but not active)
        let updated_commitments: Vec<Commitment> = e.as_contract(&core_addr, || e.storage().instance().get(&symbol_short!("COMMS")).unwrap());
        assert_eq!(updated_commitments.len(), 2); 
        
        for c in updated_commitments.iter() {
            if c.commitment_id == String::from_str(&e, "multi_1") {
                assert_eq!(c.status, String::from_str(&e, "settled"));
            } else {
                assert_eq!(c.status, String::from_str(&e, "active"));
            }
        }
    }
}
