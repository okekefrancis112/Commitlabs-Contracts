#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, String};

#[cfg(test)]
mod tests;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitmentMetadata {
    pub commitment_id: String,
    pub duration_days: u32,
    pub max_loss_percent: u32,
    pub commitment_type: String, // "safe", "balanced", "aggressive"
    pub created_at: u64,
    pub expires_at: u64,
    pub initial_amount: i128,
    pub asset_address: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitmentNFT {
    pub owner: Address,
    pub token_id: u32,
    pub metadata: CommitmentMetadata,
    pub is_active: bool,
    pub early_exit_penalty: u32,
}

#[contracttype]
pub enum DataKey {
    Admin,
    TokenCounter,
    NFTData(u32),
    Owner(u32),
    Metadata(u32),
    ActiveStatus(u32),
    TotalSupply,
}

#[contract]
pub struct CommitmentNFTContract;

mod storage {
    use super::*;

    pub fn set_admin(e: &Env, admin: &Address) {
        e.storage().instance().set(&DataKey::Admin, admin);
    }

    pub fn get_admin(e: &Env) -> Option<Address> {
        e.storage().instance().get(&DataKey::Admin)
    }

    pub fn has_admin(e: &Env) -> bool {
        e.storage().instance().has(&DataKey::Admin)
    }

    pub fn increment_token_counter(e: &Env) -> u32 {
        let mut count: u32 = e.storage().instance().get(&DataKey::TokenCounter).unwrap_or(0);
        count += 1;
        e.storage().instance().set(&DataKey::TokenCounter, &count);
        count
    }

    pub fn get_token_id(e: &Env) -> u32 {
        e.storage().instance().get(&DataKey::TokenCounter).unwrap_or(0)
    }

    pub fn set_nft_data(e: &Env, token_id: u32, nft: &CommitmentNFT) {
        e.storage().persistent().set(&DataKey::NFTData(token_id), nft);
    }

    pub fn get_nft_data(e: &Env, token_id: u32) -> Option<CommitmentNFT> {
        e.storage().persistent().get(&DataKey::NFTData(token_id))
    }

    pub fn set_owner(e: &Env, token_id: u32, owner: &Address) {
        e.storage().persistent().set(&DataKey::Owner(token_id), owner);
    }

    pub fn get_owner(e: &Env, token_id: u32) -> Option<Address> {
        e.storage().persistent().get(&DataKey::Owner(token_id))
    }

    pub fn set_metadata(e: &Env, token_id: u32, metadata: &CommitmentMetadata) {
        e.storage().persistent().set(&DataKey::Metadata(token_id), metadata);
    }

    pub fn get_metadata(e: &Env, token_id: u32) -> Option<CommitmentMetadata> {
        e.storage().persistent().get(&DataKey::Metadata(token_id))
    }

    pub fn set_active_status(e: &Env, token_id: u32, is_active: bool) {
        e.storage().persistent().set(&DataKey::ActiveStatus(token_id), &is_active);
    }

    pub fn is_active(e: &Env, token_id: u32) -> bool {
        e.storage().persistent().get(&DataKey::ActiveStatus(token_id)).unwrap_or(false)
    }

    pub fn increment_total_supply(e: &Env) {
        let mut supply: u32 = e.storage().instance().get(&DataKey::TotalSupply).unwrap_or(0);
        supply += 1;
        e.storage().instance().set(&DataKey::TotalSupply, &supply);
    }

    pub fn decrement_total_supply(e: &Env) {
        let mut supply: u32 = e.storage().instance().get(&DataKey::TotalSupply).unwrap_or(0);
        if supply > 0 {
            supply -= 1;
            e.storage().instance().set(&DataKey::TotalSupply, &supply);
        }
    }

    pub fn get_total_supply(e: &Env) -> u32 {
        e.storage().instance().get(&DataKey::TotalSupply).unwrap_or(0)
    }
}

#[contractimpl]
impl CommitmentNFTContract {
    /// Initialize the NFT contract
    pub fn initialize(e: Env, admin: Address) {
        if storage::has_admin(&e) {
            panic!("already initialized");
        }
        storage::set_admin(&e, &admin);
        e.storage().instance().set(&DataKey::TokenCounter, &0u32);
        e.storage().instance().set(&DataKey::TotalSupply, &0u32);
    }

    /// Mint a new Commitment NFT
    pub fn mint(
        e: Env,
        owner: Address,
        commitment_id: String,
        duration_days: u32,
        max_loss_percent: u32,
        commitment_type: String,
        initial_amount: i128,
        asset_address: Address,
    ) -> u32 {
        let token_id = storage::increment_token_counter(&e);
        let created_at = e.ledger().timestamp();
        let expires_at = created_at + (duration_days as u64 * 86400);

        let metadata = CommitmentMetadata {
            commitment_id,
            duration_days,
            max_loss_percent,
            commitment_type,
            created_at,
            expires_at,
            initial_amount,
            asset_address: asset_address.clone(),
        };

        let nft = CommitmentNFT {
            owner: owner.clone(),
            token_id,
            metadata: metadata.clone(),
            is_active: true,
            early_exit_penalty: 10, // Default 10% penalty
        };

        storage::set_nft_data(&e, token_id, &nft);
        storage::set_owner(&e, token_id, &owner);
        storage::set_metadata(&e, token_id, &metadata);
        storage::set_active_status(&e, token_id, true);
        storage::increment_total_supply(&e);

        token_id
    }

    /// Get NFT metadata by token_id
    pub fn get_metadata(e: Env, token_id: u32) -> CommitmentMetadata {
        storage::get_metadata(&e, token_id).expect("NFT metadata not found")
    }

    /// Get owner of NFT
    pub fn owner_of(e: Env, token_id: u32) -> Address {
        storage::get_owner(&e, token_id).expect("NFT owner not found")
    }

    /// Transfer NFT to new owner
    pub fn transfer(e: Env, from: Address, to: Address, token_id: u32) {
        from.require_auth();
        let current_owner = storage::get_owner(&e, token_id).expect("NFT owner not found");
        if current_owner != from {
            panic!("not the owner");
        }

        storage::set_owner(&e, token_id, &to);
        
        // Update NFT data as well since it contains owner
        if let Some(mut nft) = storage::get_nft_data(&e, token_id) {
            nft.owner = to.clone();
            storage::set_nft_data(&e, token_id, &nft);
        }
    }

    /// Check if NFT is active
    pub fn is_active(e: Env, token_id: u32) -> bool {
        storage::is_active(&e, token_id)
    }

    /// Mark NFT as settled (after maturity)
    pub fn settle(e: Env, token_id: u32) {
        let nft = storage::get_nft_data(&e, token_id).expect("NFT not found");
        if e.ledger().timestamp() < nft.metadata.expires_at {
            panic!("not expired yet");
        }
        storage::set_active_status(&e, token_id, false);
        
        // Update NFT data as well
        if let Some(mut nft) = storage::get_nft_data(&e, token_id) {
            nft.is_active = false;
            storage::set_nft_data(&e, token_id, &nft);
        }
    }

    /// Get total supply of NFTs
    pub fn total_supply(e: Env) -> u32 {
        storage::get_total_supply(&e)
    }

    /// Get admin address
    pub fn get_admin(e: Env) -> Address {
        storage::get_admin(&e).expect("admin not set")
    }

    /// Get current token ID
    pub fn current_token_id(e: Env) -> u32 {
        storage::get_token_id(&e)
    }
}

