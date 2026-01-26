// Integration tests for cross-contract interactions
// This file contains tests that verify interactions between Commitment NFT, Commitment Core, and Attestation Engine contracts

#![cfg(test)]

use commitment_core::{CommitmentCoreContract, CommitmentCoreContractClient, CommitmentRules};
use commitment_nft::{CommitmentNFTContract, CommitmentNFTContractClient};
use attestation_engine::{AttestationEngineContract, AttestationEngineContractClient};
use soroban_sdk::{testutils::{Address as _, Ledger}, Address, Env, String, Map};

pub struct IntegrationTestFixture {
    pub env: Env,
    pub admin: Address,
    pub owner: Address,
    pub user1: Address,
    pub verifier: Address,
    pub nft_client: CommitmentNFTContractClient<'static>,
    pub core_client: CommitmentCoreContractClient<'static>,
    pub attestation_client: AttestationEngineContractClient<'static>,
    pub asset_address: Address,
}

impl IntegrationTestFixture {
    pub fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);
        let user1 = Address::generate(&env);
        let verifier = Address::generate(&env);
        let asset_address = Address::generate(&env);

        // Deploy NFT contract
        let nft_contract_id = env.register_contract(None, CommitmentNFTContract);
        let nft_client = CommitmentNFTContractClient::new(&env, &nft_contract_id);
        nft_client.initialize(&admin);

        // Deploy Core contract  
        let core_contract_id = env.register_contract(None, CommitmentCoreContract);
        let core_client = CommitmentCoreContractClient::new(&env, &core_contract_id);
        core_client.initialize(&admin, &nft_contract_id);

        // Deploy Attestation Engine contract
        let attestation_contract_id = env.register_contract(None, AttestationEngineContract);
        let attestation_client = AttestationEngineContractClient::new(&env, &attestation_contract_id);
        attestation_client.initialize(&admin, &core_contract_id);

        IntegrationTestFixture {
            env,
            admin,
            owner,
            user1,
            verifier,
            nft_client,
            core_client,
            attestation_client,
            asset_address,
        }
    }
    
    pub fn create_test_rules(&self) -> CommitmentRules {
        CommitmentRules {
            duration_days: 30,
            max_loss_percent: 10,
            commitment_type: String::from_str(&self.env, "safe"),
            early_exit_penalty: 5,
            min_fee_threshold: 100_0000000,
        }
    }
}

// ============================================
// Cross-Contract Integration Tests
// ============================================

#[test]
#[ignore] // Requires token contract setup
fn test_create_commitment_with_attestation_flow() {
    let fixture = IntegrationTestFixture::setup();
    
    let rules = fixture.create_test_rules();

    // Step 1: Create commitment in core contract - returns u32 commitment_id
    let commitment_id: u32 = fixture.core_client.create_commitment(
        &fixture.owner,
        &1000_0000000,
        &fixture.asset_address,
        &rules,
    );

    // Verify commitment was created (commitment_id should be 1)
    assert!(commitment_id > 0);
    
    let commitment = fixture.core_client.get_commitment(&commitment_id);
    assert_eq!(commitment.owner, fixture.owner);
    assert_eq!(commitment.amount, 1000_0000000);
    assert_eq!(commitment.status, String::from_str(&fixture.env, "active"));

    // Step 2: Record attestation for the commitment
    let mut data = Map::new(&fixture.env);
    data.set(
        String::from_str(&fixture.env, "initial_value"),
        String::from_str(&fixture.env, "1000"),
    );

    fixture.attestation_client.attest(
        &commitment_id,
        &String::from_str(&fixture.env, "health_check"),
        &data,
        &fixture.verifier,
    );

    // Verify attestation was recorded
    let attestations = fixture.attestation_client.get_attestations(&commitment_id);
    assert_eq!(attestations.len(), 1);
}

#[test]
#[ignore] // Requires token contract setup
fn test_commitment_value_update_with_health_tracking() {
    let fixture = IntegrationTestFixture::setup();
    
    let rules = CommitmentRules {
        duration_days: 30,
        max_loss_percent: 10,
        commitment_type: String::from_str(&fixture.env, "balanced"),
        early_exit_penalty: 5,
        min_fee_threshold: 100_0000000,
    };

    // Create commitment
    let commitment_id: u32 = fixture.core_client.create_commitment(
        &fixture.owner,
        &1000_0000000,
        &fixture.asset_address,
        &rules,
    );

    // Update value in core contract
    fixture.core_client.update_value(&commitment_id, &1050_0000000);

    // Record health metrics in attestation engine
    fixture.attestation_client.record_fees(&fixture.admin, &commitment_id, &50_0000000);
    fixture.attestation_client.record_drawdown(&fixture.admin, &commitment_id, &1050_0000000);

    // Verify metrics
    let metrics = fixture.attestation_client.get_health_metrics(&commitment_id);
    assert_eq!(metrics.fees_generated, 50_0000000);
    // Drawdown is negative when value increased (gain instead of loss)
    assert!(metrics.drawdown_percent <= 0);

    // Verify commitment status
    let commitment = fixture.core_client.get_commitment(&commitment_id);
    assert_eq!(commitment.current_value, 1050_0000000);
    assert_eq!(commitment.status, String::from_str(&fixture.env, "active"));
}

#[test]
#[ignore] // Requires token contract setup
fn test_settlement_flow_end_to_end() {
    let fixture = IntegrationTestFixture::setup();
    
    let rules = fixture.create_test_rules();

    // Create commitment
    let commitment_id: u32 = fixture.core_client.create_commitment(
        &fixture.owner,
        &1000_0000000,
        &fixture.asset_address,
        &rules,
    );

    // Record some fees
    fixture.attestation_client.record_fees(&fixture.admin, &commitment_id, &100_0000000);

    // Fast forward past expiration
    let commitment = fixture.core_client.get_commitment(&commitment_id);
    fixture.env.ledger().with_mut(|li| {
        li.timestamp = commitment.expires_at + 1;
    });

    // Settle commitment
    fixture.core_client.settle(&commitment_id);

    // Verify commitment is settled
    let settled_commitment = fixture.core_client.get_commitment(&commitment_id);
    assert_eq!(settled_commitment.status, String::from_str(&fixture.env, "settled"));
}

#[test]
#[ignore] // Requires token contract setup
fn test_early_exit_flow_end_to_end() {
    let fixture = IntegrationTestFixture::setup();
    
    let rules = CommitmentRules {
        duration_days: 30,
        max_loss_percent: 10,
        commitment_type: String::from_str(&fixture.env, "aggressive"),
        early_exit_penalty: 10,
        min_fee_threshold: 100_0000000,
    };

    // Create commitment
    let commitment_id: u32 = fixture.core_client.create_commitment(
        &fixture.owner,
        &1000_0000000,
        &fixture.asset_address,
        &rules,
    );

    // Update value
    fixture.core_client.update_value(&commitment_id, &1100_0000000);

    // Record attestation for early exit
    let mut data = Map::new(&fixture.env);
    data.set(
        String::from_str(&fixture.env, "reason"),
        String::from_str(&fixture.env, "user_request"),
    );

    fixture.attestation_client.attest(
        &commitment_id,
        &String::from_str(&fixture.env, "early_exit"),
        &data,
        &fixture.verifier,
    );

    // Perform early exit
    fixture.core_client.early_exit(&commitment_id, &fixture.owner);

    // Verify commitment is marked as early exit
    let commitment = fixture.core_client.get_commitment(&commitment_id);
    assert_eq!(commitment.status, String::from_str(&fixture.env, "early_exit"));
}

#[test]
#[ignore] // Requires token contract setup
fn test_compliance_verification_flow() {
    let fixture = IntegrationTestFixture::setup();
    
    let rules = fixture.create_test_rules();

    // Create commitment
    let commitment_id: u32 = fixture.core_client.create_commitment(
        &fixture.owner,
        &1000_0000000,
        &fixture.asset_address,
        &rules,
    );

    // Record fees and attest - commitment in good standing
    fixture.attestation_client.record_fees(&fixture.admin, &commitment_id, &100_0000000);
    
    let mut data = Map::new(&fixture.env);
    data.set(
        String::from_str(&fixture.env, "status"),
        String::from_str(&fixture.env, "healthy"),
    );
    
    fixture.attestation_client.attest(
        &commitment_id,
        &String::from_str(&fixture.env, "health_check"),
        &data,
        &fixture.verifier,
    );

    // Verify compliance
    let is_compliant = fixture.attestation_client.verify_compliance(&commitment_id);
    assert!(is_compliant);
    
    // Calculate compliance score
    let score = fixture.attestation_client.calculate_compliance_score(&commitment_id);
    assert!(score > 0);
}

// ============================================
// Gas Optimization Tests
// ============================================

#[test]
#[ignore] // Requires token contract setup
fn test_gas_single_commitment_creation() {
    let fixture = IntegrationTestFixture::setup();
    
    let rules = fixture.create_test_rules();
    
    // Single commitment creation
    let commitment_id: u32 = fixture.core_client.create_commitment(
        &fixture.owner,
        &1000_0000000,
        &fixture.asset_address,
        &rules,
    );
    
    // Verify it was created
    assert!(commitment_id > 0);
}

#[test]
#[ignore] // Requires token contract setup
fn test_gas_multiple_operations() {
    let fixture = IntegrationTestFixture::setup();
    
    let rules = fixture.create_test_rules();
    
    // Create commitment
    let commitment_id: u32 = fixture.core_client.create_commitment(
        &fixture.owner,
        &1000_0000000,
        &fixture.asset_address,
        &rules,
    );
    
    // Multiple update operations
    fixture.core_client.update_value(&commitment_id, &1010_0000000);
    fixture.core_client.update_value(&commitment_id, &1020_0000000);
    fixture.core_client.update_value(&commitment_id, &1030_0000000);
    
    // Multiple attestation operations
    fixture.attestation_client.record_fees(&fixture.admin, &commitment_id, &10_0000000);
    fixture.attestation_client.record_fees(&fixture.admin, &commitment_id, &20_0000000);
    fixture.attestation_client.record_fees(&fixture.admin, &commitment_id, &30_0000000);
    
    // Verify final state
    let commitment = fixture.core_client.get_commitment(&commitment_id);
    assert_eq!(commitment.current_value, 1030_0000000);
    
    let metrics = fixture.attestation_client.get_health_metrics(&commitment_id);
    assert_eq!(metrics.fees_generated, 60_0000000);
}

#[test]
#[ignore] // Requires token contract setup
fn test_gas_batch_attestations() {
    let fixture = IntegrationTestFixture::setup();
    
    let rules = fixture.create_test_rules();
    
    // Create commitment
    let commitment_id: u32 = fixture.core_client.create_commitment(
        &fixture.owner,
        &1000_0000000,
        &fixture.asset_address,
        &rules,
    );
    
    // Multiple attestations
    let check_numbers = ["1", "2", "3", "4", "5"];
    for check_num in check_numbers.iter() {
        let mut data = Map::new(&fixture.env);
        data.set(
            String::from_str(&fixture.env, "check_number"),
            String::from_str(&fixture.env, check_num),
        );
        
        fixture.attestation_client.attest(
            &commitment_id,
            &String::from_str(&fixture.env, "health_check"),
            &data,
            &fixture.verifier,
        );
    }
    
    // Verify all attestations recorded
    let attestations = fixture.attestation_client.get_attestations(&commitment_id);
    assert_eq!(attestations.len(), 5);
}
