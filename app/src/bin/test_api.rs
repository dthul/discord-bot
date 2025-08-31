use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let swissrpg_client = lib::swissrpg::client::SwissRPGClient::new(
        "https://signup.swissrpg.pascalwacker.ch".to_string(),
        "authChangeMe".to_string(),
    );
    let upcoming_event_series = swissrpg_client.get_events().await?;
    println!("{:#?}", upcoming_event_series);
    Ok(())
}
