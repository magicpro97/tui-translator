use anyhow::{Context, Result};
use reqwest::header::{HeaderValue, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

/// Release asset returned by the GitHub Releases API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GithubReleaseAsset {
    /// Asset file name.
    pub name: String,
    /// Direct browser download URL.
    pub browser_download_url: String,
}

/// GitHub release metadata used by the updater.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GithubRelease {
    /// Git tag name such as `v0.1.4`.
    pub tag_name: String,
    /// Human-readable release page URL.
    pub html_url: String,
    /// Whether this release is marked as a prerelease.
    #[serde(default)]
    pub prerelease: bool,
    /// Downloadable assets attached to the release.
    #[serde(default)]
    pub assets: Vec<GithubReleaseAsset>,
}

/// Boxed future returned by [`ReleaseSource`].
pub type ReleaseFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<GithubRelease>>> + Send + 'a>>;

/// Source of GitHub release metadata for update checks.
pub trait ReleaseSource: Send + Sync {
    /// Fetch the latest release for the configured channel.
    fn latest_release<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        prerelease: bool,
    ) -> ReleaseFuture<'a>;
}

/// GitHub Releases client using the unauthenticated public API.
#[derive(Debug, Clone)]
pub struct GithubApiClient {
    client: reqwest::Client,
}

impl GithubApiClient {
    /// Create a new GitHub Releases API client.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    fn build_request(&self, url: &str) -> reqwest::RequestBuilder {
        self.client.get(url).header(
            USER_AGENT,
            HeaderValue::from_static(concat!("tui-translator/", env!("CARGO_PKG_VERSION"))),
        )
    }
}

impl Default for GithubApiClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ReleaseSource for GithubApiClient {
    fn latest_release<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        prerelease: bool,
    ) -> ReleaseFuture<'a> {
        Box::pin(async move {
            if prerelease {
                let url = format!("https://api.github.com/repos/{owner}/{repo}/releases");
                let response = self
                    .build_request(&url)
                    .send()
                    .await
                    .with_context(|| format!("request to {url} failed"))?;

                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    return Ok(None);
                }

                let response = response.error_for_status().with_context(|| {
                    format!("GitHub releases request failed for {owner}/{repo}")
                })?;
                let releases = response
                    .json::<Vec<GithubRelease>>()
                    .await
                    .context("failed to decode GitHub releases list")?;
                Ok(releases.into_iter().next())
            } else {
                let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
                let response = self
                    .build_request(&url)
                    .send()
                    .await
                    .with_context(|| format!("request to {url} failed"))?;

                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    return Ok(None);
                }

                let response = response.error_for_status().with_context(|| {
                    format!("GitHub latest-release request failed for {owner}/{repo}")
                })?;
                let release = response
                    .json::<GithubRelease>()
                    .await
                    .context("failed to decode GitHub latest release")?;
                Ok(Some(release))
            }
        })
    }
}

/// Deterministic release source used by updater tests.
#[derive(Debug, Clone, Default)]
pub struct MockReleaseSource {
    release: Option<GithubRelease>,
}

impl MockReleaseSource {
    /// Create a mock source that always returns `release`.
    pub fn new(release: Option<GithubRelease>) -> Self {
        Self { release }
    }
}

impl ReleaseSource for MockReleaseSource {
    fn latest_release<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        prerelease: bool,
    ) -> ReleaseFuture<'a> {
        let _ = (owner, repo, prerelease);
        let release = self.release.clone();
        Box::pin(async move { Ok(release) })
    }
}

#[cfg(test)]
mod tests {
    use super::{GithubApiClient, GithubRelease, GithubReleaseAsset};
    use reqwest::header::{AUTHORIZATION, USER_AGENT};

    #[test]
    fn parses_mock_release_json() {
        let json = r#"
        {
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/magicpro97/tui-translator/releases/tag/v0.2.0",
            "prerelease": true,
            "assets": [
                {
                    "name": "tui-translator-portable.zip",
                    "browser_download_url": "https://example.invalid/portable.zip"
                }
            ]
        }
        "#;

        let release: GithubRelease = serde_json::from_str(json).expect("release json should parse");
        assert_eq!(release.tag_name, "v0.2.0");
        assert!(release.prerelease);
        assert_eq!(
            release.assets,
            vec![GithubReleaseAsset {
                name: "tui-translator-portable.zip".to_string(),
                browser_download_url: "https://example.invalid/portable.zip".to_string(),
            }]
        );
    }

    #[test]
    fn request_contains_user_agent_but_no_secret_headers() {
        let client = GithubApiClient::new();
        let request = client
            .build_request("https://api.github.com/repos/magicpro97/tui-translator/releases/latest")
            .build()
            .expect("request should build");

        assert_eq!(
            request
                .headers()
                .get(USER_AGENT)
                .expect("user-agent header"),
            &reqwest::header::HeaderValue::from_static(concat!(
                "tui-translator/",
                env!("CARGO_PKG_VERSION")
            ))
        );
        assert!(request.headers().get(AUTHORIZATION).is_none());
        assert!(request.headers().get("x-api-key").is_none());
    }

    #[test]
    fn mock_release_fixture_supports_asset_lists() {
        let release = GithubRelease {
            tag_name: "v0.3.0".to_string(),
            html_url: "https://github.com/magicpro97/tui-translator/releases/tag/v0.3.0"
                .to_string(),
            prerelease: false,
            assets: vec![GithubReleaseAsset {
                name: "SHA256SUMS".to_string(),
                browser_download_url: "https://example.invalid/SHA256SUMS".to_string(),
            }],
        };

        assert_eq!(release.assets.len(), 1);
        assert_eq!(release.assets[0].name, "SHA256SUMS");
    }
}
