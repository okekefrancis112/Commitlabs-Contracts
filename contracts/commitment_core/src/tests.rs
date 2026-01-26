#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::{Address as _, Ledger}, Address, Env, String};

// Helper function to create a test commitment
fn create_test_commitment(
    e: &Env,
    commitment_id: u32,
    owner: &Address,
    amount: i128,
    current_value: i128,
    max_loss_percent: u32,
    duration_days: u32,
    created_at: u64,
) -> Commitment {
    let expires_at = created_at + (duration_days as u64 * 86400);
    
    Commitment {
        commitment_id,
        owner: owner.clone(),
        nft_token_id: 1,
        rules: CommitmentRules {
            duration_days,
            max_loss_percent,
            commitment_type: String::from_str(e, "balanced"),
            early_exit_penalty: 10,
            min_fee_threshold: 1000,
        },
        amount,
        asset_address: Address::generate(e),
        created_at,
        expires_at,
        current_value,
        status: String::from_str(e, "active"),
    }
}

// Helper to store a commitment for testing
fn store_commitment_test(e: &Env, contract_id: &Address, commitment: &Commitment) {
    e.as_contract(contract_id, || {
        set_commitment(e, commitment);
    });
}

#[test]
fn test_initialize() {
    let e = Env::default();
    let contract_id = e.register_contract(None, CommitmentCoreContract);
    let client = CommitmentCoreContractClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let nft_contract = Address::generate(&e);
    
    client.initialize(&admin, &nft_contract);
}

#[test]
#[ignore] // Requires token contract setup - to be fixed
fn test_create_commitment() {
    let e = Env::default();
    e.mock_all_auths();
    
    let admin = Address::generate(&e);
    let nft_contract = Address::generate(&e);
    let owner = Address::generate(&e);
    let asset_address = Address::generate(&e);
    
    let contract_id = e.register_contract(None, CommitmentCoreContract);
    let client = CommitmentCoreContractClient::new(&e, &contract_id);
    
    client.initialize(&admin, &nft_contract);
    
    let rules = CommitmentRules {
        duration_days: 30,
        max_loss_percent: 10,
        commitment_type: String::from_str(&e, "balanced"),
        early_exit_penalty: 5,
        min_fee_threshold: 1000,
    };
    
    let commitment_id = client.create_commitment(&owner, &1000, &asset_address, &rules);
    
    assert_eq!(commitment_id, 1);
    
    let commitment = client.get_commitment(&commitment_id);
    assert_eq!(commitment.owner, owner);
    assert_eq!(commitment.amount, 1000);
}

#[test]
#[should_panic(expected = "Invalid duration")]
fn test_validate_rules_invalid_duration() {
    let e = Env::default();
    let contract_id = e.register_contract(None, CommitmentCoreContract);

    let rules = CommitmentRules {
        duration_days: 0, // Invalid duration
        max_loss_percent: 10,
        commitment_type: String::from_str(&e, "safe"),
        early_exit_penalty: 5,
        min_fee_threshold: 100,
    };

    // Test invalid duration - should panic
    e.as_contract(&contract_id, || {
        CommitmentCoreContract::validate_rules(&e, &rules);
    });
}

#[test]
#[should_panic(expected = "Invalid max loss percent")]
fn test_validate_rules_invalid_max_loss() {
    let e = Env::default();
    let contract_id = e.register_contract(None, CommitmentCoreContract);

    let rules = CommitmentRules {
        duration_days: 30,
        max_loss_percent: 150, // Invalid max loss (> 100)
        commitment_type: String::from_str(&e, "safe"),
        early_exit_penalty: 5,
        min_fee_threshold: 100,
    };

    // Test invalid max loss percent - should panic
    e.as_contract(&contract_id, || {
        CommitmentCoreContract::validate_rules(&e, &rules);
    });
}

#[test]
#[should_panic(expected = "Invalid commitment type")]
fn test_validate_rules_invalid_type() {
    let e = Env::default();
    let contract_id = e.register_contract(None, CommitmentCoreContract);

    let rules = CommitmentRules {
        duration_days: 30,
        max_loss_percent: 10,
        commitment_type: String::from_str(&e, "invalid_type"), // Invalid type
        early_exit_penalty: 5,
        min_fee_threshold: 100,
    };

    // Test invalid commitment type - should panic
    e.as_contract(&contract_id, || {
        CommitmentCoreContract::validate_rules(&e, &rules);
    });
}

#[test]
#[ignore] // Requires token contract setup - to be fixed
fn test_get_owner_commitments() {
    let e = Env::default();
    e.mock_all_auths();
    
    let admin = Address::generate(&e);
    let nft_contract = Address::generate(&e);
    let owner = Address::generate(&e);
    let asset_address = Address::generate(&e);
    
    let contract_id = e.register_contract(None, CommitmentCoreContract);
    let client = CommitmentCoreContractClient::new(&e, &contract_id);
    
    client.initialize(&admin, &nft_contract);
    
    let rules = CommitmentRules {
        duration_days: 30,
        max_loss_percent: 10,
        commitment_type: String::from_str(&e, "balanced"),
        early_exit_penalty: 5,
        min_fee_threshold: 1000,
    };
    
    let commitment_id = client.create_commitment(&owner, &1000, &asset_address, &rules);
    
    let commitments = client.get_owner_commitments(&owner);
    assert_eq!(commitments.len(), 1);
    assert_eq!(commitments.get(0).unwrap(), commitment_id);
}

#[test]
#[ignore] // Requires token contract setup - to be fixed
fn test_settle() {
    let e = Env::default();
    e.mock_all_auths();
    
    let admin = Address::generate(&e);
    let nft_contract = Address::generate(&e);
    let owner = Address::generate(&e);
    let asset_address = Address::generate(&e);
    
    let contract_id = e.register_contract(None, CommitmentCoreContract);
    let client = CommitmentCoreContractClient::new(&e, &contract_id);
    
    client.initialize(&admin, &nft_contract);
    
    let rules = CommitmentRules {
        duration_days: 30,
        max_loss_percent: 10,
        commitment_type: String::from_str(&e, "balanced"),
        early_exit_penalty: 5,
        min_fee_threshold: 1000,
    };
    
    let commitment_id = client.create_commitment(&owner, &1000, &asset_address, &rules);
    
    // Fast forward past expiration
    let commitment = client.get_commitment(&commitment_id);
    e.ledger().with_mut(|li| {
        li.timestamp = commitment.expires_at + 1;
    });
    
    client.settle(&commitment_id);
    
    let settled = client.get_commitment(&commitment_id);
    assert_eq!(settled.status, String::from_str(&e, "settled"));
}

#[test]
#[ignore] // Requires token contract setup - to be fixed
fn test_get_total_commitments() {
    let e = Env::default();
    e.mock_all_auths();
    
    let admin = Address::generate(&e);
    let nft_contract = Address::generate(&e);
    let owner = Address::generate(&e);
    let asset_address = Address::generate(&e);
    
    let contract_id = e.register_contract(None, CommitmentCoreContract);
    let client = CommitmentCoreContractClient::new(&e, &contract_id);
    
    client.initialize(&admin, &nft_contract);
    
    // Initially zero
    let total = client.get_total_commitments();
    assert_eq!(total, 0);
    
    let rules = CommitmentRules {
        duration_days: 30,
        max_loss_percent: 10,
        commitment_type: String::from_str(&e, "balanced"),
        early_exit_penalty: 5,
        min_fee_threshold: 1000,
    };
    
    let _commitment_id = client.create_commitment(&owner, &1000, &asset_address, &rules);
    
    // Should be 1 after creation
    let total = client.get_total_commitments();
    assert_eq!(total, 1);
}

#[test]
fn test_get_admin() {
    let e = Env::default();
    let contract_id = e.register_contract(None, CommitmentCoreContract);
    let client = CommitmentCoreContractClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let nft_contract = Address::generate(&e);

    client.initialize(&admin, &nft_contract);

    let retrieved_admin = client.get_admin();
    assert_eq!(retrieved_admin, admin);
}

#[test]
fn test_get_nft_contract() {
    let e = Env::default();
    let contract_id = e.register_contract(None, CommitmentCoreContract);
    let client = CommitmentCoreContractClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let nft_contract = Address::generate(&e);

    client.initialize(&admin, &nft_contract);

    let retrieved_nft_contract = client.get_nft_contract();
    assert_eq!(retrieved_nft_contract, nft_contract);
}

#[test]
fn test_check_violations_no_violations() {
    let e = Env::default();
    let contract_id = e.register_contract(None, CommitmentCoreContract);
    let owner = Address::generate(&e);
    let commitment_id: u32 = 1;
    
    let created_at = 1000u64;
    let commitment = create_test_commitment(
        &e,
        commitment_id,
        &owner,
        1000,
        950, // 5% loss
        10,  // max 10% loss allowed
        30,
        created_at,
    );
    
    store_commitment_test(&e, &contract_id, &commitment);
    
    e.ledger().with_mut(|l| {
        l.timestamp = created_at + (15 * 86400);
    });
    
    let has_violations = e.as_contract(&contract_id, || {
        CommitmentCoreContract::check_violations(e.clone(), commitment_id)
    });
    
    assert!(!has_violations, "Should not have violations");
}

#[test]
fn test_check_violations_loss_limit_exceeded() {
    let e = Env::default();
    let contract_id = e.register_contract(None, CommitmentCoreContract);
    let owner = Address::generate(&e);
    let commitment_id: u32 = 2;
    
    let created_at = 1000u64;
    let commitment = create_test_commitment(
        &e,
        commitment_id,
        &owner,
        1000,
        850, // 15% loss - exceeds 10% limit
        10,
        30,
        created_at,
    );
    
    store_commitment_test(&e, &contract_id, &commitment);
    
    e.ledger().with_mut(|l| {
        l.timestamp = created_at + (5 * 86400);
    });
    
    let has_violations = e.as_contract(&contract_id, || {
        CommitmentCoreContract::check_violations(e.clone(), commitment_id)
    });
    
    assert!(has_violations, "Should have loss limit violation");
}

#[test]
fn test_check_violations_duration_expired() {
    let e = Env::default();
    let contract_id = e.register_contract(None, CommitmentCoreContract);
    let owner = Address::generate(&e);
    let commitment_id: u32 = 3;
    
    let created_at = 1000u64;
    let commitment = create_test_commitment(
        &e,
        commitment_id,
        &owner,
        1000,
        980, // 2% loss - within limit
        10,
        30,
        created_at,
    );
    
    store_commitment_test(&e, &contract_id, &commitment);
    
    e.ledger().with_mut(|l| {
        l.timestamp = created_at + (31 * 86400);
    });
    
    let has_violations = e.as_contract(&contract_id, || {
        CommitmentCoreContract::check_violations(e.clone(), commitment_id)
    });
    
    assert!(has_violations, "Should have duration violation");
}

#[test]
fn test_get_violation_details_no_violations() {
    let e = Env::default();
    let contract_id = e.register_contract(None, CommitmentCoreContract);
    let owner = Address::generate(&e);
    let commitment_id: u32 = 5;
    
    let created_at = 1000u64;
    let commitment = create_test_commitment(
        &e,
        commitment_id,
        &owner,
        1000,
        950, // 5% loss
        10,
        30,
        created_at,
    );
    
    store_commitment_test(&e, &contract_id, &commitment);
    
    e.ledger().with_mut(|l| {
        l.timestamp = created_at + (15 * 86400);
    });
    
    let (has_violations, loss_violated, duration_violated, loss_percent, time_remaining) = 
        e.as_contract(&contract_id, || {
            CommitmentCoreContract::get_violation_details(e.clone(), commitment_id)
        });
    
    assert!(!has_violations, "Should not have violations");
    assert!(!loss_violated, "Loss should not be violated");
    assert!(!duration_violated, "Duration should not be violated");
    assert_eq!(loss_percent, 5, "Loss percent should be 5%");
    assert!(time_remaining > 0, "Time should remain");
}

#[test]
#[ignore] // Requires token contract setup - to be fixed
fn test_update_value() {
    let e = Env::default();
    e.mock_all_auths();
    
    let admin = Address::generate(&e);
    let nft_contract = Address::generate(&e);
    let owner = Address::generate(&e);
    let asset_address = Address::generate(&e);
    
    let contract_id = e.register_contract(None, CommitmentCoreContract);
    let client = CommitmentCoreContractClient::new(&e, &contract_id);
    
    client.initialize(&admin, &nft_contract);
    
    let rules = CommitmentRules {
        duration_days: 30,
        max_loss_percent: 10,
        commitment_type: String::from_str(&e, "balanced"),
        early_exit_penalty: 5,
        min_fee_threshold: 1000,
    };
    
    let commitment_id = client.create_commitment(&owner, &1000, &asset_address, &rules);
    
    client.update_value(&commitment_id, &1100);
    
    let commitment = client.get_commitment(&commitment_id);
    assert_eq!(commitment.current_value, 1100);
}

#[test]
#[ignore] // Requires token contract setup - to be fixed
fn test_early_exit() {
    let e = Env::default();
    e.mock_all_auths();
    
    let admin = Address::generate(&e);
    let nft_contract = Address::generate(&e);
    let owner = Address::generate(&e);
    let asset_address = Address::generate(&e);
    
    let contract_id = e.register_contract(None, CommitmentCoreContract);
    let client = CommitmentCoreContractClient::new(&e, &contract_id);
    
    client.initialize(&admin, &nft_contract);
    
    let rules = CommitmentRules {
        duration_days: 30,
        max_loss_percent: 10,
        commitment_type: String::from_str(&e, "balanced"),
        early_exit_penalty: 5,
        min_fee_threshold: 1000,
    };
    
    let commitment_id = client.create_commitment(&owner, &1000, &asset_address, &rules);
    
    client.early_exit(&commitment_id, &owner);
    
    let commitment = client.get_commitment(&commitment_id);
    assert_eq!(commitment.status, String::from_str(&e, "early_exit"));
}

#[test]
#[should_panic(expected = "Commitment not found")]
fn test_check_violations_not_found() {
    let e = Env::default();
    let contract_id = e.register_contract(None, CommitmentCoreContract);
    let commitment_id: u32 = 999;
    
    e.as_contract(&contract_id, || {
        CommitmentCoreContract::check_violations(e.clone(), commitment_id)
    });
}

#[test]
fn test_check_violations_edge_case_exact_loss_limit() {
    let e = Env::default();
    let contract_id = e.register_contract(None, CommitmentCoreContract);
    let owner = Address::generate(&e);
    let commitment_id: u32 = 8;
    
    let created_at = 1000u64;
    let commitment = create_test_commitment(
        &e,
        commitment_id,
        &owner,
        1000,
        900, // Exactly 10% loss
        10,
        30,
        created_at,
    );
    
    store_commitment_test(&e, &contract_id, &commitment);
    
    e.ledger().with_mut(|l| {
        l.timestamp = created_at + (15 * 86400);
    });
    
    let has_violations = e.as_contract(&contract_id, || {
        CommitmentCoreContract::check_violations(e.clone(), commitment_id)
    });
    
    assert!(!has_violations, "Exactly at limit should not violate");
}

#[test]
fn test_check_violations_zero_amount() {
    let e = Env::default();
    let contract_id = e.register_contract(None, CommitmentCoreContract);
    let owner = Address::generate(&e);
    let commitment_id: u32 = 10;
    
    let created_at = 1000u64;
    let commitment = create_test_commitment(
        &e,
        commitment_id,
        &owner,
        0,
        0,
        10,
        30,
        created_at,
    );
    
    store_commitment_test(&e, &contract_id, &commitment);
    
    e.ledger().with_mut(|l| {
        l.timestamp = created_at + (15 * 86400);
    });
    
    let has_violations = e.as_contract(&contract_id, || {
        CommitmentCoreContract::check_violations(e.clone(), commitment_id)
    });
    
    assert!(!has_violations, "Zero amount should not cause issues");
}

