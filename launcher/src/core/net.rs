use std::time::Duration;

pub const RAW_BASE: &str = "https://raw.githubusercontent.com/HolyLightRU/HolyZapret/main/";

pub fn get_text(url: &str) -> anyhow::Result<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    Ok(agent.get(url).call()?.body_mut().read_to_string()?)
}
