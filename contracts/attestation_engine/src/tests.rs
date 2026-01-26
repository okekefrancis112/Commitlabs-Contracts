#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, testutils::Ledger as _, testutils::Events, Address, Env, String, symbol_short, vec, IntoVal, Map};
use commitment_core::{Commitment as CoreCommitment, CommitmentCoreContract, CommitmentRules as CoreCommitmentRules, DataKey};

fn store_core_commitment(
    e: &Env,
    commitment_core_id: &Address,
    commitment_id: u32,
    owner: &Address,
    amount: i128,
    current_value: i128,
    max_loss_percent: u32,
    duration_days: u32,
    created_at: u64,
) {
    let expires_at = created_at + (duration_days as u64 * 86400);
    let commitment = CoreCommitment {
        commitment_id,
        owner: owner.clone(),
        nft_token_id: 1,
        rules: CoreCommitmentRules {
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
    };

    e.as_contract(commitment_core_id, || {
        e.storage().instance().set(&DataKey::Commitment(commitment_id), &commitment);
    });
}

fn setup_test_env() -> (Env, Address, Address, Address) {
    let e = Env::default();
    let admin = Address::generate(&e);
    
    let commitment_core_id = e.register_contract(None, CommitmentCoreContract);
    let nft_contract = Address::generate(&e);
    
    e.as_contract(&commitment_core_id, || {
        CommitmentCoreContract::initialize(e.clone(), admin.clone(), nft_contract.clone());
    });
    
    let contract_id = e.register_contract(None, AttestationEngineContract);
    
    e.as_contract(&contract_id, || {
        AttestationEngineContract::initialize(e.clone(), admin.clone(), commitment_core_id.clone());
    });
    
    (e, admin, commitment_core_id, contract_id)
}

#[test]
fn test_initialize() {
    let (e, _admin, _commitment_core, contract_id) = setup_test_env();
    
    let commitment_id: u32 = 1;
    let _attestations = e.as_contract(&contract_id, || {
        AttestationEngineContract::get_attestations(e.clone(), commitment_id)
    });
}

#[test]
fn test_get_attestations_empty() {
    let (e, _admin, _commitment_core, contract_id) = setup_test_env();
    
    let commitment_id: u32 = 1;
    
    let attestations = e.as_contract(&contract_id, || {
        AttestationEngineContract::get_attestations(e.clone(), commitment_id)
    });
    
    assert_eq!(attestations.len(), 0);
}

#[test]
fn test_get_health_metrics_basic() {
    let (e, _admin, commitment_core, contract_id) = setup_test_env();
    
    let commitment_id: u32 = 1;
    let owner = Address::generate(&e);
    store_core_commitment(
        &e,
        &commitment_core,
        commitment_id,
        &owner,
        1000,
        950,
        10,
        30,
        1000,
    );

    let metrics = e.as_contract(&contract_id, || {
        AttestationEngineContract::get_health_metrics(e.clone(), commitment_id)
    });
    
    assert_eq!(metrics.commitment_id, commitment_id);
    assert!(metrics.compliance_score <= 100);
}

#[test]
fn test_get_health_metrics_drawdown_calculation() {
    let (e, _admin, commitment_core, contract_id) = setup_test_env();
    
    let commitment_id: u32 = 1;
    let owner = Address::generate(&e);
    store_core_commitment(
        &e,
        &commitment_core,
        commitment_id,
        &owner,
        1000,
        900,
        10,
        30,
        1000,
    );
    let metrics = e.as_contract(&contract_id, || {
        AttestationEngineContract::get_health_metrics(e.clone(), commitment_id)
    });
    
    assert_eq!(metrics.drawdown_percent, 10);
}

#[test]
fn test_get_health_metrics_zero_initial_value() {
    let (e, _admin, commitment_core, contract_id) = setup_test_env();
    
    let commitment_id: u32 = 1;
    let owner = Address::generate(&e);
    store_core_commitment(
        &e,
        &commitment_core,
        commitment_id,
        &owner,
        0,
        0,
        10,
        30,
        1000,
    );
    let metrics = e.as_contract(&contract_id, || {
        AttestationEngineContract::get_health_metrics(e.clone(), commitment_id)
    });
    
    assert!(metrics.drawdown_percent >= 0);
    assert_eq!(metrics.initial_value, 0);
}

#[test]
fn test_calculate_compliance_score_base() {
    let (e, _admin, commitment_core, contract_id) = setup_test_env();
    
    let commitment_id: u32 = 1;
    let owner = Address::generate(&e);
    store_core_commitment(
        &e,
        &commitment_core,
        commitment_id,
        &owner,
        1000,
        950,
        10,
        30,
        1000,
    );
    let score = e.as_contract(&contract_id, || {
        AttestationEngineContract::calculate_compliance_score(e.clone(), commitment_id)
    });
    
    assert!(score <= 100);
}

#[test]
fn test_attest_and_get_metrics() {
    let (e, admin, commitment_core, contract_id) = setup_test_env();
    
    e.ledger().with_mut(|li| li.timestamp = 12345);
    
    let commitment_id: u32 = 1;
    let owner = Address::generate(&e);
    store_core_commitment(
        &e,
        &commitment_core,
        commitment_id,
        &owner,
        1000,
        1000,
        10,
        30,
        1000,
    );
    let attestation_type = String::from_str(&e, "general");
    let mut data = Map::new(&e);
    data.set(String::from_str(&e, "note"), String::from_str(&e, "test attestation"));
    
    e.as_contract(&contract_id, || {
        AttestationEngineContract::attest(
            e.clone(),
            commitment_id,
            attestation_type.clone(),
            data.clone(),
            admin.clone(),
        );
    });
    
    let attestations = e.as_contract(&contract_id, || {
        AttestationEngineContract::get_attestations(e.clone(), commitment_id)
    });
    
    assert_eq!(attestations.len(), 1);
    assert_eq!(attestations.get(0).unwrap().attestation_type, attestation_type);
    
    let metrics = e.as_contract(&contract_id, || {
        AttestationEngineContract::get_health_metrics(e.clone(), commitment_id)
    });
    
    assert!(metrics.last_attestation > 0);
}

#[test]
fn test_record_fees() {
    let (e, admin, commitment_core, contract_id) = setup_test_env();
    e.mock_all_auths();

    let commitment_id: u32 = 1;
    let owner = Address::generate(&e);
    store_core_commitment(
        &e,
        &commitment_core,
        commitment_id,
        &owner,
        1000,
        1000,
        10,
        30,
        1000,
    );

    e.as_contract(&contract_id, || {
        AttestationEngineContract::record_fees(e.clone(), admin.clone(), commitment_id, 100);
    });

    let metrics = e.as_contract(&contract_id, || {
        AttestationEngineContract::get_health_metrics(e.clone(), commitment_id)
    });

    assert_eq!(metrics.fees_generated, 100);
}

#[test]
fn test_record_multiple_fees() {
    let (e, admin, commitment_core, contract_id) = setup_test_env();
    e.mock_all_auths();

    let commitment_id: u32 = 1;
    let owner = Address::generate(&e);
    store_core_commitment(
        &e,
        &commitment_core,
        commitment_id,
        &owner,
        1000,
        1000,
        10,
        30,
        1000,
    );

    // Call each record_fees in separate contract context to avoid auth frame issues
    e.as_contract(&contract_id, || {
        AttestationEngineContract::record_fees(e.clone(), admin.clone(), commitment_id, 50);
    });
    e.as_contract(&contract_id, || {
        AttestationEngineContract::record_fees(e.clone(), admin.clone(), commitment_id, 30);
    });
    e.as_contract(&contract_id, || {
        AttestationEngineContract::record_fees(e.clone(), admin.clone(), commitment_id, 20);
    });

    let metrics = e.as_contract(&contract_id, || {
        AttestationEngineContract::get_health_metrics(e.clone(), commitment_id)
    });

    assert_eq!(metrics.fees_generated, 100);
}

#[test]
fn test_verify_compliance() {
    let (e, _admin, commitment_core, contract_id) = setup_test_env();

    let commitment_id: u32 = 1;
    let owner = Address::generate(&e);
    store_core_commitment(
        &e,
        &commitment_core,
        commitment_id,
        &owner,
        1000,
        950,
        10,
        30,
        1000,
    );

    let is_compliant = e.as_contract(&contract_id, || {
        AttestationEngineContract::verify_compliance(e.clone(), commitment_id)
    });

    assert!(is_compliant);
}

#[test]
fn test_health_metrics_structure() {
    let (e, _admin, commitment_core, contract_id) = setup_test_env();

    let commitment_id: u32 = 1;
    let owner = Address::generate(&e);
    store_core_commitment(
        &e,
        &commitment_core,
        commitment_id,
        &owner,
        1000,
        1000,
        10,
        30,
        1000,
    );
    let metrics = e.as_contract(&contract_id, || {
        AttestationEngineContract::get_health_metrics(e.clone(), commitment_id)
    });

    assert_eq!(metrics.commitment_id, commitment_id);
    assert_eq!(metrics.current_value, 1000);
    assert_eq!(metrics.initial_value, 1000);
    assert_eq!(metrics.drawdown_percent, 0);
    assert_eq!(metrics.fees_generated, 0);
    assert_eq!(metrics.volatility_exposure, 0);
    assert_eq!(metrics.last_attestation, 0);
    assert!(metrics.compliance_score <= 100);
}
