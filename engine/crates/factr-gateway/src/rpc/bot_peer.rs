//! Registered compatible peer gateways, using the existing REST session/chat contract.
use super::*;

pub(super) fn target(base: &Path, raw: &str) -> Result<Option<Value>> {
    let raw = raw.trim().trim_start_matches('@').to_lowercase();
    let (name, profile) = raw.split_once('/').unwrap_or((&raw, "default"));
    let config: Value = std::fs::read_to_string(base.join("config.yaml"))
        .ok()
        .and_then(|s| serde_yaml::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let Some(url) = config["bot_peers"][name]["url"].as_str() else {
        anyhow::ensure!(!raw.contains('/'), "No registered peer named '{name}'");
        return Ok(None);
    };
    anyhow::ensure!(
        !profile.is_empty()
            && profile
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')),
        "invalid peer profile"
    );
    let parsed = reqwest::Url::parse(url)?;
    anyhow::ensure!(
        matches!(parsed.scheme(), "http" | "https")
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.query().is_none()
            && parsed.fragment().is_none(),
        "invalid peer URL"
    );
    let key_env = format!("FACTR_PEER_{}_KEY", name.to_uppercase().replace('-', "_"));
    anyhow::ensure!(
        std::env::var(&key_env).is_ok_and(|s| !s.trim().is_empty()),
        "missing config: peer key {key_env} is unavailable to this engine"
    );
    let base = if raw.contains('/') {
        format!("{}/p/{profile}", url.trim_end_matches('/'))
    } else {
        url.trim_end_matches('/').to_string()
    };
    Ok(Some(
        json!({"profile":profile,"handle":profile,"connection_id":format!("peer:{name}"),"peer_url":base,"peer_key_env":key_env}),
    ))
}

async fn request(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    body: Option<Value>,
) -> Result<Value> {
    let request = match body {
        Some(v) => client.post(url).json(&v),
        None => client.get(url),
    };
    let response = request.bearer_auth(key).send().await?;
    anyhow::ensure!(
        response.status().is_success(),
        "peer HTTP {}",
        response.status().as_u16()
    );
    Ok(response.json().await?)
}

pub(super) async fn deliver(envelope: &Value) -> Value {
    let outcome = async {
        let base = envelope["peer_url"].as_str().context("missing peer URL")?;
        let key = std::env::var(envelope["peer_key_env"].as_str().context("missing peer key name")?).context("missing config: peer key unavailable")?;
        // Never forward a registered peer credential to another origin through redirects.
        let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(600)).build()?;
        let listing = request(&client, &format!("{base}/api/sessions?limit=200&title=Bot%20Chat&include_hidden=1"), &key, None).await?;
        let existing = listing["data"].as_array().and_then(|rows| rows.iter().find(|r| r["title"] == "Bot Chat")).and_then(|r| r["id"].as_str()).map(str::to_string);
        let session = match existing { Some(id) => id, None => {
            let created = request(&client, &format!("{base}/api/sessions"), &key, Some(json!({"title":"Bot Chat","source":"bot_peer_dm"}))).await?;
            created["session"]["id"].as_str().or(created["id"].as_str()).context("peer returned no session id")?.to_string()
        }};
        let mut url = reqwest::Url::parse(&format!("{base}/api/sessions/"))?;
        url.path_segments_mut().map_err(|_|anyhow!("invalid peer URL"))?.pop_if_empty().push(&session).push("chat");
        let origin = format!("{:x}", Sha256::digest(bot_message::install_root().to_string_lossy().as_bytes()));
        let answer = request(&client, url.as_str(), &key, Some(json!({"message":envelope["message"],"author":{"id":format!("bot:{}@{}",envelope["from_profile"].as_str().unwrap_or("default"),&origin[..12]),"name":envelope["from_handle"],"is_bot":true}}))).await?;
        if answer["object"] == "factr.session.chat.queued" {
            bail!("peer accepted the message in its open Bot Chat; the answer remains there. Do not resend");
        }
        Ok::<_,anyhow::Error>(answer["message"]["content"].as_str().unwrap_or("").to_string())
    }.await;
    match outcome {
        Ok(reply) => json!({"reply":reply}),
        Err(error) => {
            let error = error.to_string();
            json!({"error":error,"reason":bot_chat_deliver::classify(&error)})
        }
    }
}
