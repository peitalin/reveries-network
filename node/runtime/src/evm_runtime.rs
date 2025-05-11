use color_eyre::Result;
use std::sync::Arc;
use tracing::{info, warn, error};
use std::str::FromStr;
use dotenv::dotenv;

use alloy::{
    providers::{
        Provider,
        RootProvider,
    },
    rpc::types::{
        BlockNumberOrTag,
        Filter,
        request::TransactionRequest,
        TransactionReceipt,
    },
    network::{Ethereum}, // Network trait and Ethereum network marker
    transports::http::reqwest,
};
use alloy_primitives::{U256, Address};
use alloy_rpc_client::RpcClient;
use alloy_signer_local::PrivateKeySigner;


#[derive(Clone, Debug)]
pub struct EvmConfig {
    pub rpc_url: String,
    pub chain_id: Option<u64>,
}

impl EvmConfig {
    pub fn new() -> Result<Self> {
        dotenv().ok();
        // let rpc_url = std::env::var("EVM_RPC_URL")
        //     .map_err(|_| color_eyre::eyre::eyre!("EVM_RPC_URL not set in environment"))?;
        // let chain_id_str = std::env::var("EVM_CHAIN_ID").ok();

        let rpc_url = "https://rpc.sepolia.org".to_string();
        let chain_id = Some(11155111);

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
                rpc_url: "https://rpc.sepolia.org".to_string(),
                chain_id: Some(11155111),
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

    Ok(())
}
