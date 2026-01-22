#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, Address, Env, String};

#[test]
fn test_initialize() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let contract_id = e.register_contract(None, CommitmentNFTContract);
    let client = CommitmentNFTContractClient::new(&e, &contract_id);
    
    client.initialize(&admin);
    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.total_supply(), 0);
}

#[test]
fn test_mint() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let owner = Address::generate(&e);
    let asset = Address::generate(&e);
    let contract_id = e.register_contract(None, CommitmentNFTContract);
    let client = CommitmentNFTContractClient::new(&e, &contract_id);
    
    client.initialize(&admin);
    
    let token_id = client.mint(
        &owner,
        &String::from_str(&e, "commitment-1"),
        &30,
        &10,
        &String::from_str(&e, "balanced"),
        &1000,
        &asset,
    );
    
    assert_eq!(token_id, 1);
    assert_eq!(client.owner_of(&1), owner);
    assert_eq!(client.total_supply(), 1);
    assert_eq!(client.is_active(&1), true);
    
    let metadata = client.get_metadata(&1);
    assert_eq!(metadata.commitment_id, String::from_str(&e, "commitment-1"));
    assert_eq!(metadata.duration_days, 30);
}

#[test]
fn test_transfer() {
    let e = Env::default();
    let _from = Address::generate(&e);
    let _to = Address::generate(&e);
    let _contract_id = e.register_contract(None, CommitmentNFTContract);
    
    // TODO: Test transfer
}

