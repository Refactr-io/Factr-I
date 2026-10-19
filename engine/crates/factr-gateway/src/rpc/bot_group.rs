//! The member engine owns each room/thread session pointer, independently of the Python session store.
use super::*;
static MINT: Mutex<()> = Mutex::const_new(());

fn key(profile: &str, title: &str) -> String {
    format!("bot_group:{profile}:{:x}", Sha256::digest(title.as_bytes()))
}

impl Conn {
    pub(super) async fn bot_group_ensure(self: &Arc<Self>, p: &Value) -> Result<Value, RpcError> {
        let own = crate::profile::current()
            .name
            .unwrap_or_else(|| "default".into());
        if p["profile"].as_str().is_some_and(|name| name != own) {
            return Err(RpcError::params(
                "group session reached the wrong member engine",
            ));
        }
        let title = p["title"]
            .as_str()
            .filter(|s| s.starts_with("Group: ") && s.len() <= 512)
            .ok_or_else(|| RpcError::params("a Group title is required"))?;
        let _mint = MINT.lock().await;
        let store = entries_or_log(&self.config.home)
            .ok_or_else(|| RpcError::internal(anyhow!("session store unavailable")))?;
        let key = key(&own, title);
        let existing = store
            .setting(&key)
            .filter(|id| factr_base::session::session_exists(id))
            .or_else(|| {
                p["session_id"]
                    .as_str()
                    .filter(|id| {
                        factr_base::session::Session::load(id).is_ok_and(|s| {
                            s.custom_title.as_deref().or(s.title.as_deref()) == Some(title)
                        })
                    })
                    .map(str::to_string)
            });
        let id = if let Some(id) = existing {
            id
        } else {
            let created = Box::pin(self.dispatch("session.create", &json!({"title":title,"hidden":true,"room_plumbing":true,"follow_profile_config":true}))).await?;
            let id = created["session_id"]
                .as_str()
                .ok_or_else(|| RpcError::internal(anyhow!("engine returned no group session")))?
                .to_string();
            store.set_setting(&key, &id).map_err(RpcError::internal)?;
            id
        };
        store.set_setting(&key, &id).map_err(RpcError::internal)?;
        self.ensure_attached(&id)
            .await
            .map_err(RpcError::internal)?;
        Ok(json!({"session_id":id,"stored_session_id":id,"session_key":id,"title":title}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn members_and_threads_never_share_a_pointer() {
        assert_ne!(key("a", "Group: room · one"), key("b", "Group: room · one"));
        assert_ne!(key("a", "Group: room · one"), key("a", "Group: room · two"));
    }
}
