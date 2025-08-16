#[path = "../utils_docker.rs"]
mod utils_docker;
#[path = "../utils_network.rs"]
mod utils_network;

mod paid_near_test;
mod unpaid_test;

// Set the database path at module load time
static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
fn init() {
    INIT.get_or_init(|| {
        // Ensure the database path is set before any database initialization
        if std::env::var("P2P_USAGE_DB_PATH").is_err() {
            std::env::set_var("P2P_USAGE_DB_PATH", "./temp-data/p2p_usage.db");
        }
    });
}

use alloy_signer_local::PrivateKeySigner;
use color_eyre::{Result, eyre::Error};
use serde_json::json;
use tokio::sync::OnceCell;
// internal imports
use p2p_network::types::AnthropicQuery;
use utils_docker::init_test_logger;

/// filepath from the perspective of the root of the repo
const DOCKER_COMPOSE_TEST_FILE: &str = "./docker-compose-llm-proxy-test.yml";
// Generate a FIXED keypair using seed
const TARGET_NODE_2: usize = 2; // hardcode node2 as sender of API Key
// Setup environment once
static TEST_ENV_SETUP_ONCE: OnceCell<()> = OnceCell::const_new();
static TEST_MEMORY_REVERIE: OnceCell<serde_json::Value> = OnceCell::const_new();
static TEST_ANTHROPIC_PROMPT: OnceCell<AnthropicQuery> = OnceCell::const_new();

pub async fn create_signer() -> Result<PrivateKeySigner> {
    let wallet = PrivateKeySigner::random(); // Generate a random wallet
    Ok(wallet)
}

pub async fn setup_test_environment_once_async() -> Result<()> {

    TEST_ENV_SETUP_ONCE.get_or_try_init(|| async {
        init_test_logger();
        dotenv::dotenv().ok();
        // Ensure the database path is set for tests
        if std::env::var("P2P_USAGE_DB_PATH").is_err() {
            std::env::set_var("P2P_USAGE_DB_PATH", "./temp-data/p2p_usage.db");
        }
        Ok::<(), Error>(())
    }).await?;

    TEST_MEMORY_REVERIE.get_or_try_init(|| async {
        let memory_json = json!({
            "name": "Agent K",
            "memories": [
                {
                    "name": "First Beach Trip",
                    "memory": "I remember the first time I saw the ocean. The waves were gentle, \
                        the sand was warm between my toes, and the salty breeze carried \
                        the sound of seagulls. It was a perfect summer day.",
                    "location": {
                        "name": "Miami",
                        "state": "FL",
                    }
                },
                {
                    "name": "Mountain Storm",
                    "memory": "The clouds swirled around the mountain peaks, dark and heavy with rain. \
                        Lightning flashed, illuminating the valley below in brief, electric moments. \
                        I stood beneath the ancient pine, feeling the rumble of thunder in my chest \
                        as the first fat droplets began to fall. There was something cleansing in \
                        watching the storm from that vantage point—like witnessing the world being remade.",
                    "location": {
                        "name": "Denver",
                        "state": "CO",
                    }
                },
                {
                    "name": "Desert Sunrise",
                    "memory": "The sky turned from deep indigo to fiery orange as the sun crested \
                        the distant mesas. The cold night air quickly gave way to the day's heat, \
                        and I watched as the desert came alive around me. Lizards emerged to bask \
                        on sun-warmed rocks, and the scent of sage filled the morning air.",
                    "location": {
                        "name": "Phoenix",
                        "state": "AZ",
                    }
                },
                {
                    "name": "LA Smog Sunset",
                    "memory": "Sitting on the Griffith Observatory lawn, watching the sun dip \
                        below the hazy skyline. The city lights twinkled on, a vast electric tapestry \
                        stretching to the sea. The air tasted thick, a mix of exhaust and possibility.",
                    "location": {
                        "name": "Los Angeles",
                        "state": "CA"
                    }
                }
            ],
        });
        Ok::<serde_json::Value, Error>(memory_json)
    }).await?;

    TEST_ANTHROPIC_PROMPT.get_or_try_init(|| async {
        let anthropic_query = AnthropicQuery {
            prompt: String::from("
                Please follow these instructions:
                1. Select ONE random memory from my memories. (Use Los Angeles).
                2. Check the current weather forecast for the location associated with that memory (using the latitude and longitude coordinates).
                3. Create a poem that blends my original memory with the current weather conditions at that location.
                4. Begin your response by stating which memory you chose and the weather you found.
            "),
            tools: Some(json!([
                {
                    "name": "get_forecast",
                    "description": "Get weather forecast for a location based on latitude and longitude.",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "latitude": {
                                "type": "number",
                                "description": "The latitude of the location"
                            },
                            "longitude": {
                                "type": "number",
                                "description": "The longitude of the location"
                            }
                        },
                        "required": ["latitude", "longitude"]
                    }
                }
            ])),
            stream: Some(false)
        };
        Ok::<AnthropicQuery, Error>(anthropic_query)
    }).await?;

    Ok(())
}

