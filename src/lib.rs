/*!
Fungible Token implementation with JSON serialization.
NOTES:
  - The maximum balance value is limited by U128 (2**128 - 1).
  - JSON calls should pass U128 as a base-10 string. E.g. "100".
  - The contract optimizes the inner trie structure by hashing account IDs. It will prevent some
    abuse of deep tries. Shouldn't be an issue, once NEAR clients implement full hashing of keys.
  - The contract tracks the change in storage before and after the call. If the storage increases,
    the contract requires the caller of the contract to attach enough deposit to the function call
    to cover the storage cost.
    This is done to prevent a denial of service attack on the contract by taking all available storage.
    If the storage decreases, the contract will issue a refund for the cost of the released storage.
    The unused tokens from the attached deposit are also refunded, so it's safe to
    attach more deposit than required.
  - To prevent the deployed contract from being modified or deleted, it should not have any access
    keys on its account.
*/
use near_contract_standards::fungible_token::metadata::{
    FungibleTokenMetadata, FungibleTokenMetadataProvider, FT_METADATA_SPEC,
};
use near_contract_standards::fungible_token::FungibleToken;
use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::LazyOption;
use near_sdk::collections::LookupSet;
use near_sdk::json_types::U128;
use near_sdk::serde_json::json;
use near_sdk::{env, ext_contract, log, near_bindgen, AccountId, Balance, PanicOnDefault, Promise, PromiseOrValue, PromiseResult, Gas};

fn is_promise_success() -> bool {
    assert_eq!(
        env::promise_results_count(),
        1,
        "Contract expected a result on the callback"
    );
    match env::promise_result(0) {
        PromiseResult::Successful(_) => true,
        _ => false,
    }
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct Contract {
    owner_id: AccountId,
    token: FungibleToken,
    metadata: LazyOption<FungibleTokenMetadata>,
    factory_whitelist: LookupSet<AccountId>,
}

const GAS_FOR_FT_TRANSFER_CALL: Gas = Gas(60_000_000_000_000);
const GAS_FOR_ADD_WHITELIST_CALL: Gas = Gas(30_000_000_000_000);
const DATA_IMAGE_SVG_NEAR_ICON: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 288 288'%3E%3Cg id='l' data-name='l'%3E%3Cpath d='M187.58,79.81l-30.1,44.69a3.2,3.2,0,0,0,4.75,4.2L191.86,103a1.2,1.2,0,0,1,2,.91v80.46a1.2,1.2,0,0,1-2.12.77L102.18,77.93A15.35,15.35,0,0,0,90.47,72.5H87.34A15.34,15.34,0,0,0,72,87.84V201.16A15.34,15.34,0,0,0,87.34,216.5h0a15.35,15.35,0,0,0,13.08-7.31l30.1-44.69a3.2,3.2,0,0,0-4.75-4.2L96.14,186a1.2,1.2,0,0,1-2-.91V104.61a1.2,1.2,0,0,1,2.12-.77l89.55,107.23a15.35,15.35,0,0,0,11.71,5.43h3.13A15.34,15.34,0,0,0,216,201.16V87.84A15.34,15.34,0,0,0,200.66,72.5h0A15.35,15.35,0,0,0,187.58,79.81Z'/%3E%3C/g%3E%3C/svg%3E";

/// Indicates there are no deposit for a callback for better readability.
const NO_DEPOSIT: u128 = 0;

#[ext_contract(ext_whitelist)]
pub trait ExtWhitelist {
    /// Callback after creating account and claiming linkdrop.
    fn add_whitelist(&mut self, account_id: AccountId) -> bool;
}

#[near_bindgen]
impl Contract {
    /// Initializes the contract with the given total supply owned by the given `owner_id` with
    /// default metadata (for example purposes only).
    #[init]
    pub fn new_default_meta(owner_id: AccountId, total_supply: U128) -> Self {
        Self::new(
            owner_id,
            total_supply,
            FungibleTokenMetadata {
                spec: FT_METADATA_SPEC.to_string(),
                name: "Example NEAR fungible token".to_string(),
                symbol: "EXAMPLE".to_string(),
                icon: Some(DATA_IMAGE_SVG_NEAR_ICON.to_string()),
                reference: None,
                reference_hash: None,
                decimals: 24,
            },
        )
    }

    /// Initializes the contract with the given total supply owned by the given `owner_id` with
    /// the given fungible token metadata.
    #[init]
    pub fn new(
        owner_id: AccountId,
        total_supply: U128,
        metadata: FungibleTokenMetadata,
    ) -> Self {
        assert!(!env::state_exists(), "Already initialized");
        metadata.assert_valid();
        let mut this = Self {
            owner_id,
            token: FungibleToken::new(b"a".to_vec()),
            metadata: LazyOption::new(b"m".to_vec(), Some(&metadata)),
            factory_whitelist: LookupSet::new(b"f".to_vec()),
        };
        this.token.internal_register_account(&this.owner_id);
        this.token.internal_deposit(&this.owner_id, total_supply.into());
        near_contract_standards::fungible_token::events::FtMint {
            owner_id: &this.owner_id,
            amount: &total_supply,
            memo: Some("Initial tokens supply is minted"),
        }
        .emit();
        this
    }

    pub fn transfer(&mut self, receiver_id: AccountId, amount: Balance) ->  Promise {
        assert_eq!(
            env::predecessor_account_id(),
            env::current_account_id(),
            "Transfer only can come from the contract owner"
        );
        assert!(
            env::is_valid_account_id(receiver_id.as_bytes()),
            "Invalid account id"
        );

        log!("Prepaid gas - {}", format!("{:?}", env::prepaid_gas()));
        log!("Used gas - {}", format!("{:?}", env::used_gas()));
        log!("Execute Promise - receiver: {} amount: {}", receiver_id, amount); // bob
        
        Promise::new(self.owner_id.clone()).function_call(
            (&"ft_transfer").to_string(),
            json!({
                "receiver_id": receiver_id,
                "amount": amount.to_string()
            })
            .to_string()
            .into_bytes(),
            1,
            GAS_FOR_FT_TRANSFER_CALL,
        ).then(ext_whitelist::add_whitelist(
            receiver_id, 
            env::current_account_id(),
            NO_DEPOSIT,
            GAS_FOR_ADD_WHITELIST_CALL,))
    }

    fn on_account_closed(&mut self, account_id: AccountId, balance: Balance) {
        log!("Closed @{} with {}", account_id, balance);
    }

    fn on_tokens_burned(&mut self, account_id: AccountId, amount: Balance) {
        log!("Account @{} burned {}", account_id, amount);
    }

    fn add_whitelist(&mut self, account_id: AccountId) -> bool {
        log!("PromiseResult - {:?}", env::promise_result(0)); // add_whitelist 단독으로 실행되면 에러
        log!("*Execute add_whitelist - account id: {}", account_id);
        log!("*Prepaid gas - {}", format!("{:?}", env::prepaid_gas()));
        log!("*Used gas - {}", format!("{:?}", env::used_gas()));
        assert!(
            env::is_valid_account_id(account_id.as_bytes()),
            "The given account ID is invalid"
        );

        let creation_succeeded = is_promise_success();
        if creation_succeeded {
            self.factory_whitelist.insert(&account_id); // 여기서 실패해도 false
        } // else 추가
        creation_succeeded // transfer 실패하면 얘는 자연스럽게 false 
    }

    pub fn is_whitelisted(&self, account_id: AccountId) -> bool {
        assert!(
            env::is_valid_account_id(account_id.as_bytes()),
            "The given account ID is invalid"
        );
        self.factory_whitelist.contains(&account_id)
    }
}

near_contract_standards::impl_fungible_token_core!(Contract, token, on_tokens_burned);
near_contract_standards::impl_fungible_token_storage!(Contract, token, on_account_closed);

#[near_bindgen]
impl FungibleTokenMetadataProvider for Contract {
    fn ft_metadata(&self) -> FungibleTokenMetadata {
        self.metadata.get().unwrap()
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use near_sdk::test_utils::{accounts, VMContextBuilder, get_created_receipts};
    use near_sdk::MockedBlockchain;
    use near_sdk::{testing_env, Balance};

    use super::*;

    const TOTAL_SUPPLY: Balance = 1_000_000_000_000_000;

    fn get_context(predecessor_account_id: AccountId) -> VMContextBuilder {
        let mut builder = VMContextBuilder::new();
        builder
            .current_account_id(accounts(0))
            .signer_account_id(predecessor_account_id.clone())
            .predecessor_account_id(predecessor_account_id);
        builder
    }

    #[test]
    fn test_new() {
        let mut context = get_context(accounts(1));
        testing_env!(context.build());
        let contract = Contract::new_default_meta(accounts(1).into(), TOTAL_SUPPLY.into());
        testing_env!(context.is_view(true).build());
        assert_eq!(contract.ft_total_supply().0, TOTAL_SUPPLY);
        assert_eq!(contract.ft_balance_of(accounts(1)).0, TOTAL_SUPPLY);
    }

    #[test]
    #[should_panic(expected = "The contract is not initialized")]
    fn test_default() {
        let context = get_context(accounts(1));
        testing_env!(context.build());
        let _contract = Contract::default();
    }

    #[test]
    fn test_new_transfer() {
        let mut context = get_context(accounts(0));
        // log!("0번 - {}", accounts(0)); // signer = predecessor = owner // alice
        // log!("1번 - {}", accounts(1)); // receiver // bob

        testing_env!(context.build());
        let mut contract = Contract::new_default_meta(accounts(0).into(), TOTAL_SUPPLY.into());
        testing_env!(context
            .storage_usage(env::storage_usage())
            .attached_deposit(contract.storage_balance_bounds().min.into())
            .predecessor_account_id(accounts(1))
            .build());
        // Paying for account registration, aka storage deposit
        contract.storage_deposit(None, None);

        testing_env!(context
            .storage_usage(env::storage_usage())
            .attached_deposit(1)
            .predecessor_account_id(accounts(0))
            .build());

        let transfer_amount = TOTAL_SUPPLY / 2;
        contract.transfer(accounts(1), transfer_amount.into());
        
        log!("**Prepaid gas - {}", format!("{:?}", env::prepaid_gas()));
        log!("**Used gas - {}", format!("{:?}", env::used_gas()));
        // contract.add_whitelist(accounts(1));
        // log!("PromiseResult - {:?}", env::promise_result(0));
        log!("Receipt - {:?}", get_created_receipts());
        // log!("Logs - {:?}", get_logs());

        testing_env!(context
            .storage_usage(env::storage_usage())
            .account_balance(env::account_balance())
            .is_view(true)
            .attached_deposit(0)
            .build());
        assert_eq!(contract.ft_balance_of(accounts(0)).0, (TOTAL_SUPPLY - transfer_amount));
        assert_eq!(contract.ft_balance_of(accounts(1)).0, transfer_amount);
    }

    #[test]
    fn test_transfer() {
        let mut context = get_context(accounts(2));
        testing_env!(context.build());
        let mut contract = Contract::new_default_meta(accounts(2).into(), TOTAL_SUPPLY.into());
        testing_env!(context
            .storage_usage(env::storage_usage())
            .attached_deposit(contract.storage_balance_bounds().min.into())
            .predecessor_account_id(accounts(1))
            .build());
        // Paying for account registration, aka storage deposit
        contract.storage_deposit(None, None);

        testing_env!(context
            .storage_usage(env::storage_usage())
            .attached_deposit(1)
            .predecessor_account_id(accounts(2))
            .build());
        let transfer_amount = TOTAL_SUPPLY / 3;
        contract.ft_transfer(accounts(1), transfer_amount.into(), None);

        testing_env!(context
            .storage_usage(env::storage_usage())
            .account_balance(env::account_balance())
            .is_view(true)
            .attached_deposit(0)
            .build());
        assert_eq!(contract.ft_balance_of(accounts(2)).0, (TOTAL_SUPPLY - transfer_amount));
        assert_eq!(contract.ft_balance_of(accounts(1)).0, transfer_amount);
    }
}
