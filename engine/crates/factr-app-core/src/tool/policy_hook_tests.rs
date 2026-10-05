//! The provider-facing tool-policy hook (hosted tools such as `image_generation`).

#[test]
fn provider_hook_follows_the_session_tool_policy_for_the_hosted_image_tool() {
    use std::collections::HashSet;
    let set = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<HashSet<_>>();
    let s = "hosted-image-policy";
    super::set_session_tool_policy(s, Some(set(&["bash", "read"])), set(&[]));
    assert!(!factr_base::tool_policy::tool_allowed(Some(s), "image_generate", "image_gen"));
    super::set_session_tool_policy(s, Some(set(&["bash", "image_gen"])), set(&[]));
    assert!(factr_base::tool_policy::tool_allowed(Some(s), "image_generate", "image_gen"));
    super::set_session_tool_policy(s, None, set(&["image_generate"]));
    assert!(!factr_base::tool_policy::tool_allowed(Some(s), "image_generate", "image_gen"));
    // No enabled_toolsets policy, or no policy at all: unchanged (allowed).
    super::set_session_tool_policy(s, None, set(&[]));
    assert!(factr_base::tool_policy::tool_allowed(Some(s), "image_generate", "image_gen"));
    super::clear_session_tool_policy(s);
    assert!(factr_base::tool_policy::tool_allowed(Some(s), "image_generate", "image_gen"));
    assert!(factr_base::tool_policy::tool_allowed(None, "image_generate", "image_gen"));
}
