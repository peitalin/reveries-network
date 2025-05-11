use color_eyre::Result;
use std::sync::Arc;
use tracing::{info, warn, error};
use std::str::FromStr;
use dotenv::dotenv;

use alloy::{
    network::{Ethereum, TransactionBuilder},
    primitives::{address, Address, U256},
    providers::{Provider, ProviderBuilder, RootProvider},
    rpc::{
        client::RpcClient,
        types::{
            request::TransactionRequest,
            TransactionReceipt,
        },
    },
    signers::local::PrivateKeySigner,
    sol,
};

// ERC20 interface for reading balance
sol! {
    #[sol(rpc)]
    contract ERC20 {
        function balanceOf(address owner) public view returns (uint256 balance);
    }
}

// WETH9 interface for depositing ETH
sol! {
    #[sol(rpc)]
    contract WETH9 {
        function deposit() public payable;
        function balanceOf(address owner) public view returns (uint256 balance);
    }
}

#[derive(Clone, Debug)]
pub struct EvmConfig {
    pub rpc_url: String,
    pub chain_id: Option<u64>,
}

impl EvmConfig {
    pub fn new() -> Result<Self> {
        dotenv().ok();
        let rpc_url = std::env::var("BASE_SEPOLIA_RPC_URL").unwrap_or_else(|_| {
            warn!("BASE_SEPOLIA_RPC_URL not set, using default https://base-sepolia.gateway.tenderly.co");
            "https://base-sepolia.gateway.tenderly.co".to_string()
        });
        let chain_id = std::env::var("EVM_CHAIN_ID")
            .ok()
            .and_then(|id_str| id_str.parse::<u64>().ok())
            .or(Some(84532));

        Ok(Self {
            rpc_url,
            chain_id,
        })
    }
}

impl Default for EvmConfig {
    fn default() -> Self {
        EvmConfig::new().unwrap_or_else(|e| {
            warn!("Failed to load EvmConfig from env, using defaults: {}", e);
            Self {
                rpc_url: "https://base-sepolia.gateway.tenderly.co".to_string(),
                chain_id: Some(84532),
            }
        })
    }
}


#[derive(Clone, Debug)]
pub struct EvmRuntime {
    provider: Arc<RootProvider<Ethereum>>,
    config: EvmConfig,
}

impl EvmRuntime {
    pub async fn new(config: EvmConfig) -> Result<Self> {
        info!("Initializing EvmRuntime with Alloy-rs...");
        info!("EVM RPC URL: {}", config.rpc_url);
        if let Some(chain_id) = config.chain_id {
            info!("EVM Chain ID: {}", chain_id);
        }

        let rpc_url_parsed = reqwest::Url::parse(&config.rpc_url)?;
        let rpc_client = RpcClient::new_http(rpc_url_parsed);
        let provider = RootProvider::<Ethereum>::new(rpc_client);

        Ok(Self {
            provider: Arc::new(provider),
            config,
        })
    }

    pub async fn get_balance(&self, address_str: &str) -> Result<U256> {
        let address = Address::from_str(address_str)?;
        info!("Fetching EVM balance for address: {:?}", address);
        let balance = self.provider.get_balance(address).await?;
        Ok(balance)
    }

    /// Reads the ERC20 token balance for a given owner address from a specified contract.
    pub async fn get_erc20_balance(&self, contract_address: Address, owner_address: Address) -> Result<U256> {
        info!("Reading ERC20 balance from contract: {:?} for owner: {:?}", contract_address, owner_address);
        let erc20_contract = ERC20::new(contract_address, self.provider.clone());
        let result = erc20_contract.balanceOf(owner_address).call().await?;
        Ok(result)
    }

    /// Deposits ETH to the WETH9 contract and returns the transaction receipt.
    /// The signer_hex_key is the private key in hex format.
    pub async fn deposit_weth(
        &self,
        signer_hex_key: &str,
        weth_contract_address: Address,
        amount: U256,
    ) -> Result<TransactionReceipt> {
        info!("Depositing {} WEI to WETH contract: {:?}", amount, weth_contract_address);
        let signer: PrivateKeySigner = signer_hex_key.parse()?;
        info!("Using signer address: {:?}", signer.address());

        let provider_with_signer = ProviderBuilder::new()
            .wallet(signer.clone())
            .connect_client(RpcClient::new_http(self.config.rpc_url.parse()?));

        let weth_contract = WETH9::new(weth_contract_address, provider_with_signer);
        let tx_builder = weth_contract.deposit().value(amount);

        info!("Sending WETH deposit transaction...");
        let pending_tx = tx_builder.send().await?;
        info!("WETH deposit transaction sent, hash: {:?}", pending_tx.tx_hash());
        let receipt = pending_tx.get_receipt().await?;
        info!("WETH deposit transaction confirmed, receipt: {:?}", receipt);
        Ok(receipt)
    }

    /// Sends an ETH transfer and returns the transaction receipt.
    /// The signer_hex_key is the private key in hex format.
    pub async fn send_eth_transfer(
        &self,
        signer_hex_key: &str,
        to: Address,
        value: U256,
    ) -> Result<TransactionReceipt> {
        info!("Sending {} WEI to address: {:?}", value, to);
        let signer: PrivateKeySigner = signer_hex_key.parse()?;
        info!("Using signer address: {:?}", signer.address());

        let provider_with_signer = ProviderBuilder::new()
            .wallet(signer.clone())
            .connect_client(RpcClient::new_http(self.config.rpc_url.parse()?));

        let mut tx = TransactionRequest::default()
            .with_to(to)
            .value(value);

        if let Some(chain_id_val) = self.config.chain_id {
            tx.chain_id = Some(chain_id_val);
        }
        // Nonce and gas are typically filled by the provider or its fillers.

        info!("Sending ETH transfer transaction: {:?}", tx);
        let pending_tx = provider_with_signer.send_transaction(tx).await?;
        info!("ETH transfer transaction sent, hash: {:?}", pending_tx.tx_hash());
        let receipt = pending_tx.get_receipt().await?;
        info!("ETH transfer transaction confirmed, receipt: {:?}", receipt);
        Ok(receipt)
    }

//     pub async fn send_transaction(
//         &self,
//         wallet: &PrivateKeySigner,
//         tx_request: TransactionRequest,
//     ) -> Result<TxHash> {
//         info!("Preparing to send EVM transaction from: {:?} to: {:?}", wallet.address(), tx_request.to);

//         let mut filled_tx = tx_request.clone();

//         if filled_tx.chain_id.is_none() {
//             if let Some(chain_id) = self.config.chain_id {
//                 filled_tx.chain_id = Some(chain_id);
//             }
//         }

//         let signer = PrivateKeySigner::from(wallet.clone());

//         let provider_with_signer = ProviderBuilder::new()
//             .filler(NonceFiller::default())
//             .filler(GasFiller::default())
//             .filler(ChainIdFiller::default())
//             .signer(signer)
//             .provider(self.provider.clone());

//         info!("Sending filled EVM transaction: {:?}", filled_tx);
//         let pending_tx = provider_with_signer.send_transaction(filled_tx).await?;
//         let tx_hash = *pending_tx.tx_hash();
//         info!("Transaction sent, hash: {:?}", tx_hash);

//         Ok(tx_hash)
//     }

//     pub async fn read_contract_state(
//         &self,
//         contract_address: Address,
//         call_data: AlloyBytes
//     ) -> Result<AlloyBytes> {
//         let tx_req = TransactionRequest {
//             to: Some(contract_address),
//             input: call_data.into(),
//             ..Default::default()
//         };

//         info!("Reading EVM contract state for {:?} with data: {:?}", contract_address, tx_req.input);
//         let result_bytes = Provider::call(&*self.provider, &tx_req, Some(BlockNumberOrTag::Latest.into())).await?;
//         Ok(result_bytes)
//     }

}

pub async fn evm_example() -> Result<()> {
    let config = EvmConfig::default();
    let runtime = EvmRuntime::new(config).await?;

    let eth_address_str = "0x000000000000000000000000000000000000dEaD";
    println!("getting balance for EVM Address: {:?}", eth_address_str);
    let eth_balance = runtime.get_balance(eth_address_str).await?;
    info!("EVM balance for {}: {}", eth_address_str, eth_balance);

    // Example for get_erc20_balance (requires a known ERC20 token on Sepolia)
    // You'll need to replace with an actual token and owner address.
    // let weth_on_sepolia = address!("0x7b79995e5f793A07Bc00c21412e50Ecae098E7f9"); // Example WETH on Sepolia
    // let some_owner = address!("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"); // Vitalik's address for example
    // match runtime.get_erc20_balance(weth_on_sepolia, some_owner).await {
    //     Ok(balance) => info!("WETH balance for {}: {}", some_owner, balance),
    //     Err(e) => error!("Failed to get WETH balance: {}", e),
    // }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::utils::{parse_ether, format_ether};
    use std::env;

    fn setup_test_logger() {
        // Ensure logs are visible during tests.
        // Allow `RUST_LOG=info` or similar to override.
        let _ = tracing_subscriber::fmt().with_env_filter("info,alloy_rpc_client=off").try_init();
    }

    fn get_mandatory_env_var(var_name: &str) -> Result<String> {
        env::var(var_name).map_err(|_| color_eyre::eyre::eyre!("Mandatory environment variable {} not set", var_name))
    }

    #[tokio::test]
    async fn test_get_erc20_balance_of_weth() -> Result<()> {
        setup_test_logger();
        dotenv().ok();
        let config = EvmConfig::default(); // Uses BASE_SEPOLIA_RPC_URL or default Base Sepolia
        let runtime = EvmRuntime::new(config).await?;

        let weth_contract_address_str = get_mandatory_env_var("BASE_SEPOLIA_WETH_ADDRESS")
            .unwrap_or_else(|_| "0x4200000000000000000000000000000000000006".to_string()); // Default Base Sepolia WETH
        let weth_contract_address = Address::from_str(&weth_contract_address_str)?;

        // Using a known address that might have WETH or a burn address.
        // For a real test, you'd use an address you control and have deposited WETH to.
        let owner_address_str = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"; // Vitalik's address
        let owner_address = Address::from_str(owner_address_str)?;

        info!("Testing get_erc20_balance for WETH: {:?} on owner: {:?}", weth_contract_address, owner_address);
        let balance = runtime.get_erc20_balance(weth_contract_address, owner_address).await?;
        info!("WETH balance of {}: {}", owner_address_str, balance);
        // We can't assert a specific balance easily, so we just check if the call succeeded.
        Ok(())
    }

    #[tokio::test]
    #[ignore] // This test involves state change and requires a funded private key
    async fn test_deposit_weth_on_sepolia() -> Result<()> {
        setup_test_logger();
        dotenv().ok();

        let signer_hex_key = get_mandatory_env_var("BASE_SEPOLIA_PRIVATE_KEY")?;
        let weth_contract_address_str = get_mandatory_env_var("BASE_SEPOLIA_WETH_ADDRESS")
            .unwrap_or_else(|_| "0x4200000000000000000000000000000000000006".to_string());

        let config = EvmConfig::default();
        let runtime = EvmRuntime::new(config).await?;

        let weth_contract_address = Address::from_str(&weth_contract_address_str)?;
        let deposit_amount = parse_ether("0.0001").unwrap(); // Deposit a very small amount

        info!("Testing WETH deposit of {} to {}", format_ether(deposit_amount), weth_contract_address_str);
        let receipt = runtime.deposit_weth(
            &signer_hex_key,
            weth_contract_address,
            deposit_amount
        ).await?;

        assert_eq!(receipt.status(), true, "WETH deposit transaction failed");
        info!("WETH deposit successful. Transaction hash: {:?}", receipt.transaction_hash);
        Ok(())
    }

    #[tokio::test]
    #[ignore] // This test involves state change and requires a funded private key
    async fn test_send_eth_transfer_on_sepolia() -> Result<()> {
        setup_test_logger();
        dotenv().ok();

        let signer_hex_key = get_mandatory_env_var("BASE_SEPOLIA_PRIVATE_KEY")?;
        let recipient_address_str = get_mandatory_env_var("BASE_SEPOLIA_TEST_RECIPIENT_ADDRESS")
            .unwrap_or_else(|_| "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045".to_string()); // Default to a burn address

        let config = EvmConfig::default();
        let runtime = EvmRuntime::new(config).await?;

        let recipient_address = Address::from_str(&recipient_address_str)?;
        let transfer_amount = parse_ether("0.00001").unwrap(); // Transfer a very small amount

        info!("Testing ETH transfer of {} to {}", format_ether(transfer_amount), recipient_address_str);
        let receipt = runtime.send_eth_transfer(
            &signer_hex_key,
            recipient_address,
            transfer_amount
        ).await?;

        assert_eq!(receipt.status(), true, "ETH transfer transaction failed");
        info!("ETH transfer successful. Transaction hash: {:?}", receipt.transaction_hash);
        Ok(())
    }
}
