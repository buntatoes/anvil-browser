//! Built-in tracker, ad-network, telemetry, fingerprinter, and malicious domain shield.
//!
//! Evaluates outgoing URLs before any DNS lookup or TCP/TLS socket connection is opened.

use crate::{is_same_site, parse_url};

const BLOCKED_DOMAIN_SUFFIXES: &[&str] = &[
    "doubleclick.net",
    "adnxs.com",
    "adsrvr.org",
    "adsystem.com",
    "criteo.com",
    "criteo.net",
    "taboola.com",
    "outbrain.com",
    "scorecardresearch.com",
    "quantserve.com",
    "hotjar.com",
    "mixpanel.com",
    "segment.io",
    "amplitude.com",
    "omtrdc.net",
    "demdex.net",
    "rubiconproject.com",
    "pubmatic.com",
    "openx.net",
    "casalemedia.com",
    "moatads.com",
    "chartbeat.com",
    "nr-data.net",
    "mouseflow.com",
    "fullstory.com",
    "clarity.ms",
    "adserver.example.net",
    "tracker.example.org",
    "telemetry.example.org",
    "malware-test.example.org",
    "phishing-test.example.org",
];

const BLOCKED_HOST_PREFIXES: &[&str] = &[
    "ads.",
    "ad.",
    "adservice.",
    "adserver.",
    "telemetry.",
    "tracker.",
    "tracking.",
    "pixel.",
    "beacon.",
    "metrics.",
    "analytics.",
    "fingerprinter.",
];

const BLOCKED_PATH_PATTERNS: &[&str] = &[
    "/pagead/",
    "/adserver/",
    "/ads/banner",
    "/pixel.gif",
    "/beacon.js",
    "/fingerprint.js",
    "/telemetry/collect",
    "/track.js",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShieldBlockReason {
    BlockedDomain(String),
    BlockedTrackerSubdomain(String),
    BlockedTrackingPath(String),
}

impl std::fmt::Display for ShieldBlockReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BlockedDomain(dom) => write!(f, "blocked tracker/ad domain ({dom})"),
            Self::BlockedTrackerSubdomain(host) => {
                write!(f, "blocked tracker/telemetry subdomain ({host})")
            }
            Self::BlockedTrackingPath(path) => {
                write!(f, "blocked tracking script/pixel path ({path})")
            }
        }
    }
}

/// Stateful tracker, ad, and malicious domain shield.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkShield {
    pub enabled: bool,
    custom_blocked_domains: Vec<String>,
    allowed_domains: Vec<String>,
    blocked_total: u64,
}

impl Default for NetworkShield {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkShield {
    pub fn new() -> Self {
        Self {
            enabled: true,
            custom_blocked_domains: Vec::new(),
            allowed_domains: Vec::new(),
            blocked_total: 0,
        }
    }

    pub fn blocked_total(&self) -> u64 {
        self.blocked_total
    }

    pub fn record_block(&mut self) {
        self.blocked_total = self.blocked_total.saturating_add(1);
    }

    pub fn add_blocked_domain(&mut self, domain: &str) {
        let clean = domain.trim().trim_start_matches('.').to_ascii_lowercase();
        if !clean.is_empty() && !self.custom_blocked_domains.contains(&clean) {
            self.custom_blocked_domains.push(clean);
        }
    }

    pub fn add_allowed_domain(&mut self, domain: &str) {
        let clean = domain.trim().trim_start_matches('.').to_ascii_lowercase();
        if !clean.is_empty() && !self.allowed_domains.contains(&clean) {
            self.allowed_domains.push(clean);
        }
    }

    /// Inspect `target_url` (optionally relative to `initiator_url`) and return a block reason
    /// if the URL matches a known tracker, ad network, telemetry collector, or malicious host.
    pub fn inspect_url(
        &self,
        target_url: &str,
        initiator_url: Option<&str>,
    ) -> Option<ShieldBlockReason> {
        if !self.enabled {
            return None;
        }
        let Ok(parsed) = parse_url(target_url) else {
            return None;
        };
        let host = parsed.host.to_ascii_lowercase();

        for allow in &self.allowed_domains {
            if host == *allow || host.ends_with(&format!(".{allow}")) {
                return None;
            }
        }

        for dom in BLOCKED_DOMAIN_SUFFIXES
            .iter()
            .copied()
            .chain(self.custom_blocked_domains.iter().map(String::as_str))
        {
            if host == dom || host.ends_with(&format!(".{dom}")) {
                return Some(ShieldBlockReason::BlockedDomain(dom.to_string()));
            }
        }

        for prefix in BLOCKED_HOST_PREFIXES {
            if host.starts_with(prefix) {
                return Some(ShieldBlockReason::BlockedTrackerSubdomain(host));
            }
        }

        let path_low = parsed.path.to_ascii_lowercase();
        for pat in BLOCKED_PATH_PATTERNS {
            if path_low.contains(pat) {
                // Allow same-site first-party assets unless they explicitly match a tracking script/pixel
                let cross_site = initiator_url.is_some_and(|init| !is_same_site(init, target_url));
                if cross_site
                    || pat.contains("beacon")
                    || pat.contains("fingerprint")
                    || pat.contains("pixel")
                    || pat.contains("pagead")
                    || pat.contains("adserver")
                    || pat.contains("telemetry")
                {
                    return Some(ShieldBlockReason::BlockedTrackingPath((*pat).to_string()));
                }
            }
        }

        None
    }

    /// Inspect `target_url` and increment the shield's blocked counter if blocked.
    pub fn check_and_count(
        &mut self,
        target_url: &str,
        initiator_url: Option<&str>,
    ) -> Option<ShieldBlockReason> {
        let reason = self.inspect_url(target_url, initiator_url);
        if reason.is_some() {
            self.record_block();
        }
        reason
    }
}

/// Stateless check against the default built-in tracker & malicious domain ruleset.
pub fn is_url_blocked_by_default_shield(
    target_url: &str,
    initiator_url: Option<&str>,
) -> Option<ShieldBlockReason> {
    NetworkShield::new().inspect_url(target_url, initiator_url)
}
