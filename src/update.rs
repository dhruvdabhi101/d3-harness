use std::time::Duration;

fn newer_release(tag: &str, current: &str) -> Option<String> {
    fn version(value: &str) -> Option<[u64; 3]> {
        let mut parts = value.trim_start_matches('v').split('.');
        let result = [
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
        ];
        if parts.next().is_some() {
            return None;
        }
        Some(result)
    }
    (version(tag)? > version(current)?).then(|| tag.to_string())
}

pub async fn check() -> Option<String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("d3/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    let release: serde_json::Value = client
        .get("https://api.github.com/repos/dhruvdabhi101/d3-harness/releases/latest")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    newer_release(
        release.get("tag_name")?.as_str()?,
        env!("CARGO_PKG_VERSION"),
    )
}

#[cfg(test)]
mod tests {
    use super::newer_release;

    #[test]
    fn reports_only_newer_stable_versions() {
        assert_eq!(newer_release("v0.2.0", "0.1.0"), Some("v0.2.0".into()));
        assert_eq!(newer_release("v0.1.0", "0.1.0"), None);
        assert_eq!(newer_release("v0.2.0-rc.1", "0.1.0"), None);
        assert_eq!(newer_release("invalid", "0.1.0"), None);
    }
}
