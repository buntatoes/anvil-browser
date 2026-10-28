// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//
// Directly links and runs Brave Browser's unmodified open-source `adblock-rust` crate
// (`adblock`, https://github.com/brave/adblock-rust, MPL-2.0).

//! Brave Shields & Adblock integration powered directly by Brave's unmodified `adblock` crate.
//!
//! Wraps [`adblock::Engine`], [`adblock::lists::FilterSet`], [`adblock::request::Request`],
//! [`adblock::resources::Resource`], and [`adblock::cosmetic_filter_cache::UrlSpecificResources`]
//! for `anvil-net` and `anvil-browser`.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

pub use adblock::Engine as BraveAdblockEngine;
pub use adblock::blocker::BlockerResult as BraveBlockerResult;
pub use adblock::cosmetic_filter_cache::UrlSpecificResources as BraveUrlSpecificResources;
pub use adblock::lists::{FilterFormat, FilterSet, ParseOptions, RuleTypes};
pub use adblock::request::{Request as BraveRequest, RequestError as BraveRequestError};
pub use adblock::resources::{MimeType, PermissionMask, Resource, ResourceStorage, ResourceType};

use crate::parse_url;

/// 64-bit hash type re-exported from `adblock::utils::Hash`.
pub type Hash = adblock::utils::Hash;

/// Reason a network request was blocked by Brave Shields / `NetworkShield`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShieldBlockReason {
    /// Host matched a built-in or custom tracker/advertising domain rule.
    TrackerDomain(String),
    /// URL path or query matched a telemetry/pixel/ad-beacon pattern.
    TelemetryPath(String),
    /// Cross-site third-party subresource blocked while strict third-party blocking is enabled.
    ThirdPartySubresource {
        initiator_host: String,
        target_host: String,
    },
    /// Blocked by a rule in Brave's `adblock::Engine`.
    FilterRule(String),
}

impl fmt::Display for ShieldBlockReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TrackerDomain(host) => write!(f, "shield blocked tracker host {host}"),
            Self::TelemetryPath(pat) => write!(f, "shield blocked telemetry pattern {pat}"),
            Self::ThirdPartySubresource {
                initiator_host,
                target_host,
            } => write!(
                f,
                "shield blocked cross-site subresource {target_host} from {initiator_host}"
            ),
            Self::FilterRule(rule) => write!(f, "shield blocked by filter rule {rule}"),
        }
    }
}

/// Brave Shields operating mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShieldsMode {
    /// Blocks third-party ads, trackers, cryptominers, and applies cosmetic filtering & param stripping.
    #[default]
    Standard,
    /// Blocks both first-party and third-party ad/tracker patterns, strict subresources, and generic cosmetics.
    Aggressive,
    /// Shields disabled globally.
    Disabled,
}

impl ShieldsMode {
    /// Human-readable label for UI display.
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Aggressive => "Aggressive",
            Self::Disabled => "Disabled",
        }
    }

    /// Lowercase slug for URLs and profile persistence.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Aggressive => "aggressive",
            Self::Disabled => "disabled",
        }
    }

    /// Parse a `ShieldsMode` from a slug.
    pub fn from_slug(slug: &str) -> Self {
        match slug.trim().to_ascii_lowercase().as_str() {
            "aggressive" | "strict" => Self::Aggressive,
            "disabled" | "off" | "down" => Self::Disabled,
            _ => Self::Standard,
        }
    }

    /// Cycle through `Standard` -> `Aggressive` -> `Disabled` -> `Standard`.
    pub fn cycle_next(self) -> Self {
        match self {
            Self::Standard => Self::Aggressive,
            Self::Aggressive => Self::Disabled,
            Self::Disabled => Self::Standard,
        }
    }
}

/// Resource type of a network request passed to `adblock::request::Request::new`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RequestType {
    Document,
    Subdocument,
    Script,
    Stylesheet,
    Image,
    Media,
    Font,
    XmlHttpRequest,
    Fetch,
    Ping,
    Beacon,
    Websocket,
    Object,
    #[default]
    Other,
}

impl RequestType {
    /// Convert to the exact CPT string expected by `adblock::request::Request::new`.
    pub fn as_cpt_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Subdocument => "subdocument",
            Self::Script => "script",
            Self::Stylesheet => "stylesheet",
            Self::Image => "image",
            Self::Media => "media",
            Self::Font => "font",
            Self::XmlHttpRequest | Self::Fetch => "xmlhttprequest",
            Self::Ping => "ping",
            Self::Beacon => "beacon",
            Self::Websocket => "websocket",
            Self::Object => "object",
            Self::Other => "other",
        }
    }
}

/// Bitmask helper retained for API compatibility.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetworkFilterMask(pub u32);

/// Parsed network filter wrapper retained for API compatibility.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkFilter {
    pub raw_line: String,
}

/// Parsed cosmetic filter wrapper retained for API compatibility.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CosmeticFilter {
    pub raw_line: String,
}

/// Cosmetic filter resources computed for a specific page URL via `adblock::Engine`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CosmeticResources {
    /// CSS selectors that should be styled with `{ display: none !important; }`.
    pub hide_selectors: Vec<String>,
    /// Custom `(selector, declarations)` rules from `:style(...)` procedural actions.
    pub style_rules: Vec<(String, String)>,
    /// Selectors explicitly unhidden on this domain via `#@#`.
    pub exceptions: HashSet<String>,
    /// Injected scriptlet code from `+js(...)` rules.
    pub injected_script: String,
    /// True if a `$generichide` exception matched this URL.
    pub generichide: bool,
}

impl CosmeticResources {
    /// Build a ready-to-inject CSS stylesheet for `anvil-style`.
    pub fn to_stylesheet(&self) -> String {
        if self.hide_selectors.is_empty() && self.style_rules.is_empty() {
            return String::new();
        }
        let mut css = String::new();
        for sel in &self.hide_selectors {
            css.push_str(sel);
            css.push_str(" { display: none !important; }\n");
        }
        for (sel, decls) in &self.style_rules {
            css.push_str(sel);
            css.push_str(" { ");
            css.push_str(decls);
            if !decls.trim_end().ends_with(';') {
                css.push(';');
            }
            css.push_str(" }\n");
        }
        css
    }
}

/// Web-accessible `$redirect` surrogate storage backed by `adblock::resources::Resource`.
#[derive(Clone, Default)]
pub struct RedirectResourceStorage {
    resources: Vec<Resource>,
}

impl fmt::Debug for RedirectResourceStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedirectResourceStorage")
            .field("resource_count", &self.resources.len())
            .finish()
    }
}

impl RedirectResourceStorage {
    /// Build the default set of Brave / uBlock Origin web-accessible redirect resources.
    pub fn new() -> Self {
        let mut s = Self {
            resources: Vec::new(),
        };
        s.add_text_resource(
            "noop.js",
            &["noopjs"],
            MimeType::ApplicationJavascript,
            "(function() {})();",
        );
        s.add_text_resource("noop.txt", &["nooptext", "empty"], MimeType::TextPlain, "");
        s.add_text_resource("noop.css", &["noopcss"], MimeType::TextCss, "");
        s.add_text_resource(
            "noop.html",
            &["noopframe"],
            MimeType::TextHtml,
            "<!DOCTYPE html><html><head><title></title></head><body></body></html>",
        );
        s.add_text_resource(
            "google-analytics_analytics.js",
            &["google-analytics.com/analytics.js", "ga.js"],
            MimeType::ApplicationJavascript,
            "/* GoogleAnalyticsObject */ function ga() { return 1; } var gaLoaded = 1;",
        );
        s.add_text_resource(
            "googletagmanager_gtm.js",
            &["googletagmanager.com/gtm.js"],
            MimeType::ApplicationJavascript,
            "var dataLayer = []; var gtmLoaded = 1;",
        );
        s.add_text_resource(
            "googlesyndication_adsbygoogle.js",
            &["googlesyndication.com/adsbygoogle.js"],
            MimeType::ApplicationJavascript,
            "var adsbygoogle = []; var adsbygoogleLoaded = 1;",
        );
        s.add_text_resource(
            "scorecardresearch_beacon.js",
            &["scorecardresearch.com/beacon.js"],
            MimeType::ApplicationJavascript,
            "function COMSCORE() { return 1; } var comscoreLoaded = 1;",
        );
        s
    }

    fn add_text_resource(
        &mut self,
        name: &str,
        aliases: &[&str],
        mime: MimeType,
        plain_text: &str,
    ) {
        self.resources.push(Resource {
            name: name.to_string(),
            aliases: aliases.iter().map(|a| (*a).to_string()).collect(),
            kind: ResourceType::Mime(mime),
            content: base64_encode(plain_text.as_bytes()),
            dependencies: Vec::new(),
            permission: PermissionMask::default(),
        });
    }

    /// Return the slice of `adblock::resources::Resource` items.
    pub fn as_slice(&self) -> &[Resource] {
        &self.resources
    }
}

/// Complete decision returned by [`NetworkShield::check_network_request`] (powered by `adblock::blocker::BlockerResult`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockerResult {
    /// True if the request should be blocked.
    pub matched: bool,
    /// True if the match came from an `$important` rule (ignoring exceptions).
    pub important: bool,
    /// Block reason if `matched` is true.
    pub reason: Option<ShieldBlockReason>,
    /// Matched exception rule string, if an `@@` rule allowed the request.
    pub exception: Option<String>,
    /// Decoded `(content_type, body)` from `adblock::blocker::BlockerResult::redirect` data URL.
    pub redirect_resource: Option<(String, String)>,
    /// Rewritten URL if `$removeparam` modified the URL.
    pub rewritten_url: Option<String>,
}

/// Built-in tracker and advertising domains blocked by default.
const DEFAULT_BLOCKED_DOMAINS: &[&str] = &[
    "doubleclick.net",
    "googlesyndication.com",
    "googleadservices.com",
    "google-analytics.com",
    "googletagmanager.com",
    "googletagservices.com",
    "adservice.google.com",
    "pagead2.googlesyndication.com",
    "analytics.google.com",
    "facebook.net",
    "connect.facebook.net",
    "graph.facebook.com",
    "pixel.facebook.com",
    "ads-twitter.com",
    "analytics.twitter.com",
    "t.co",
    "scorecardresearch.com",
    "sb.scorecardresearch.com",
    "quantserve.com",
    "pixel.quantserve.com",
    "adnxs.com",
    "ib.adnxs.com",
    "criteo.com",
    "criteo.net",
    "static.criteo.net",
    "taboola.com",
    "trc.taboola.com",
    "outbrain.com",
    "widgets.outbrain.com",
    "rubiconproject.com",
    "fastlane.rubiconproject.com",
    "pubmatic.com",
    "ads.pubmatic.com",
    "openx.net",
    "casalemedia.com",
    "amazon-adsystem.com",
    "aax.amazon-adsystem.com",
    "c.amazon-adsystem.com",
    "moatads.com",
    "z.moatads.com",
    "adsafeprotected.com",
    "pixel.adsafeprotected.com",
    "hotjar.com",
    "static.hotjar.com",
    "mouseflow.com",
    "fullstory.com",
    "rs.fullstory.com",
    "clarity.ms",
    "www.clarity.ms",
    "mixpanel.com",
    "api.mixpanel.com",
    "segment.io",
    "api.segment.io",
    "cdn.segment.com",
    "amplitude.com",
    "api.amplitude.com",
    "heapanalytics.com",
    "newrelic.com",
    "bam.nr-data.net",
    "js-agent.newrelic.com",
    "bugsnag.com",
    "notify.bugsnag.com",
    "loggly.com",
    "kissmetrics.com",
    "statcounter.com",
    "chartbeat.com",
    "static.chartbeat.com",
    "omtrdc.net",
    "2o7.net",
    "demdex.net",
    "everesttech.net",
    "krxd.net",
    "cdn.krxd.net",
    "bluekai.com",
    "tags.bluekai.com",
    "exelator.com",
    "rlcdn.com",
    "tapad.com",
    "adsrvr.org",
    "match.adsrvr.org",
    "mathtag.com",
    "bidswitch.net",
    "smartadserver.com",
    "teads.tv",
    "sharethrough.com",
    "indexexchange.com",
    "3lift.com",
    "tlx.3lift.com",
    "yieldmo.com",
    "zemanta.com",
    "revcontent.com",
    "mgid.com",
    "adform.net",
    "track.adform.net",
    "serving-sys.com",
    "flashtalking.com",
    "innovid.com",
    "celtra.com",
    "coinhive.com",
    "coin-hive.com",
    "crypto-loot.com",
    "jsecoin.com",
    "minero.cc",
    "webmine.cz",
];

/// Built-in telemetry path/query substrings blocked on third-party requests.
const DEFAULT_TELEMETRY_PATTERNS: &[&str] = &[
    "/pagead/",
    "/ads/ga-audiences",
    "/g/collect?",
    "/j/collect?",
    "/ccm/collect",
    "/collect",
    "/pixel.gif",
    "/fingerprint.js",
    "adserver.",
    "/pixel/track",
    "/beacon.js",
    "/fbevents.js",
    "/adsbygoogle.js",
    "/gpt.js",
    "/analytics.js",
    "/gtm.js",
    "utm_medium=ad",
    "/adserver/",
    "/adframe.",
    "/adbanner/",
    "/popunder.",
    "/cryptominer.",
];

/// Default Brave Shields / uBlock Origin / EasyList / EasyPrivacy ruleset compiled into `adblock::Engine`.
const DEFAULT_BRAVE_ABP_RULES: &[&str] = &[
    "! Brave Shields & uBlock Origin Static Rules",
    "||doubleclick.net^$third-party",
    "||googlesyndication.com^$third-party",
    "||googleadservices.com^$third-party",
    "||google-analytics.com^$third-party",
    "||googletagmanager.com^$third-party",
    "||googletagservices.com^$third-party",
    "||pagead2.googlesyndication.com/pagead/js/adsbygoogle.js$script,redirect=googlesyndication_adsbygoogle.js",
    "||www.google-analytics.com/analytics.js$script,redirect=google-analytics_analytics.js",
    "||google-analytics.com/analytics.js$script,redirect=google-analytics_analytics.js",
    "||www.googletagmanager.com/gtm.js$script,redirect=googletagmanager_gtm.js",
    "||sb.scorecardresearch.com/beacon.js$script,redirect=scorecardresearch_beacon.js",
    "@@||google-analytics.com/plugins/$script",
    "||connect.facebook.net/*/fbevents.js$script,redirect=noop.js",
    "||adnxs.com^$third-party",
    "||criteo.com^$third-party",
    "||criteo.net^$third-party",
    "||taboola.com^$third-party",
    "||outbrain.com^$third-party",
    "||rubiconproject.com^$third-party",
    "||pubmatic.com^$third-party",
    "||openx.net^$third-party",
    "||casalemedia.com^$third-party",
    "||amazon-adsystem.com^$third-party",
    "||moatads.com^$third-party",
    "||adsafeprotected.com^$third-party",
    "||quantserve.com^$third-party",
    "||scorecardresearch.com^$third-party",
    "||hotjar.com^$third-party",
    "||clarity.ms^$third-party",
    "||fullstory.com^$third-party",
    "||mouseflow.com^$third-party",
    "||coinhive.com^$important",
    "||coin-hive.com^$important",
    "||crypto-loot.com^$important",
    "||minero.cc^$important",
    "||slopeaota.com^$document",
    "||trafficstars.com^$document",
    "||traffichunt.com^$document",
    "! Query Parameter Stripping ($removeparam)",
    "$removeparam=utm_source",
    "$removeparam=utm_medium",
    "$removeparam=utm_campaign",
    "$removeparam=utm_term",
    "$removeparam=utm_content",
    "$removeparam=utm_id",
    "$removeparam=fbclid",
    "$removeparam=gclid",
    "$removeparam=dclid",
    "$removeparam=msclkid",
    "$removeparam=twclid",
    "$removeparam=mc_eid",
    "$removeparam=_hsenc",
    "$removeparam=_hsmi",
    "$removeparam=mkt_tok",
    "$removeparam=igshid",
    "$removeparam=oly_enc_id",
    "$removeparam=oly_anon_id",
    "$removeparam=vero_id",
    "$removeparam=rb_clickid",
    "! Cosmetic Element Hiding Rules (EasyList / uBlock Origin / Brave Shields)",
    "##.ad-banner",
    "##.ad-container",
    "##.ad-slot",
    "##.ad-unit",
    "##.ad-wrapper",
    "##.adsbox",
    "##.adsbygoogle",
    "##.advertisement",
    "##.sponsored-content",
    "##.sponsored-post",
    "##.taboola-widget",
    "##.outbrain-widget",
    "##.cookie-consent-banner",
    "###ad-banner",
    "###ad-container",
    "###ad-leaderboard",
    "###ad-sidebar",
    "###google_ads_frame",
    "###masthead-ad",
    "##ins.adsbygoogle",
    "##div[data-ad-slot]",
    "##div[data-google-query-id]",
    "google.*###tads",
    "google.*###tadsb",
    "google.*###bottomads",
    "google.*##.commercial-unit-desktop-top",
    "google.*##.commercial-unit-desktop-rhs",
    "google.*##.ads-ad",
    "youtube.com##.masthead-ad-control",
    "youtube.com##.ad-div",
    "youtube.com##.pyv-afc-ads-container",
    "youtube.com###promotion-shelf",
    "youtube.com##.ytp-ad-progress",
    "youtube.com###masthead-ad",
];

/// Default generic class names probed against `adblock::Engine::hidden_class_id_selectors`.
const DEFAULT_GENERIC_AD_CLASSES: &[&str] = &[
    "ad-banner",
    "ad-container",
    "ad-slot",
    "ad-unit",
    "ad-wrapper",
    "adsbox",
    "adsbygoogle",
    "advertisement",
    "sponsored-content",
    "sponsored-post",
    "taboola-widget",
    "outbrain-widget",
    "cookie-consent-banner",
];

/// Default generic element IDs probed against `adblock::Engine::hidden_class_id_selectors`.
const DEFAULT_GENERIC_AD_IDS: &[&str] = &[
    "ad-banner",
    "ad-container",
    "ad-leaderboard",
    "ad-sidebar",
    "google_ads_frame",
    "masthead-ad",
];

/// Build a compiled `adblock::Engine` from the given filter rules and redirect resources.
fn build_brave_engine(rules: &[String], resources: &RedirectResourceStorage) -> BraveAdblockEngine {
    let mut filter_set = FilterSet::new(true);
    let opts = ParseOptions::default();
    filter_set.add_filter_list(rules.join("\n"), opts);
    let mut engine = BraveAdblockEngine::new_with_filter_set(filter_set);
    engine.use_resources(resources.as_slice().iter().cloned());
    engine
}

/// Shared default `adblock::Engine` built once for fast `NetworkShield::new()` construction.
static BASE_BRAVE_ENGINE: OnceLock<(
    Arc<BraveAdblockEngine>,
    Vec<String>,
    RedirectResourceStorage,
)> = OnceLock::new();

fn base_brave_engine() -> &'static (
    Arc<BraveAdblockEngine>,
    Vec<String>,
    RedirectResourceStorage,
) {
    BASE_BRAVE_ENGINE.get_or_init(|| {
        let mut all_rules: Vec<String> =
            Vec::with_capacity(DEFAULT_BLOCKED_DOMAINS.len() + DEFAULT_BRAVE_ABP_RULES.len());
        for &domain in DEFAULT_BLOCKED_DOMAINS {
            all_rules.push(format!("||{domain}^"));
        }
        for &rule in DEFAULT_BRAVE_ABP_RULES {
            all_rules.push(rule.to_string());
        }
        let resources = RedirectResourceStorage::new();
        let engine = Arc::new(build_brave_engine(&all_rules, &resources));
        (engine, all_rules, resources)
    })
}

/// Brave Shields & Adblock engine backed directly by Brave's `adblock::Engine` (`adblock-rust`).
#[derive(Clone)]
pub struct NetworkShield {
    enabled: bool,
    mode: ShieldsMode,
    block_third_party_subresources: bool,
    blocked_domains: Vec<String>,
    telemetry_patterns: Vec<String>,
    disabled_sites: HashSet<String>,
    all_rules: Vec<String>,
    custom_rules: Vec<String>,
    custom_class_probes: Vec<String>,
    custom_id_probes: Vec<String>,
    resources: RedirectResourceStorage,
    engine: Arc<BraveAdblockEngine>,
    blocked_total: u64,
    params_stripped_total: u64,
}

impl fmt::Debug for NetworkShield {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NetworkShield")
            .field("enabled", &self.enabled)
            .field("mode", &self.mode)
            .field("network_rule_count", &self.network_rule_count())
            .field("cosmetic_rule_count", &self.cosmetic_rule_count())
            .field("blocked_total", &self.blocked_total)
            .field("params_stripped_total", &self.params_stripped_total)
            .finish()
    }
}

impl Default for NetworkShield {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkShield {
    /// Create a new `NetworkShield` powered by Brave's `adblock::Engine`.
    pub fn new() -> Self {
        let (engine, base_rules, resources) = base_brave_engine();
        Self {
            enabled: true,
            mode: ShieldsMode::Standard,
            block_third_party_subresources: false,
            blocked_domains: DEFAULT_BLOCKED_DOMAINS
                .iter()
                .map(|s| s.to_ascii_lowercase())
                .collect(),
            telemetry_patterns: DEFAULT_TELEMETRY_PATTERNS
                .iter()
                .map(|s| s.to_ascii_lowercase())
                .collect(),
            disabled_sites: HashSet::new(),
            all_rules: base_rules.clone(),
            custom_rules: Vec::new(),
            custom_class_probes: Vec::new(),
            custom_id_probes: Vec::new(),
            resources: resources.clone(),
            engine: Arc::clone(engine),
            blocked_total: 0,
            params_stripped_total: 0,
        }
    }

    /// Borrow the underlying unmodified `adblock::Engine` from Brave's `adblock-rust`.
    pub fn brave_engine(&self) -> &BraveAdblockEngine {
        &self.engine
    }

    /// Return whether the shield is globally enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled && self.mode != ShieldsMode::Disabled
    }

    /// Enable or disable the shield globally.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.mode = ShieldsMode::Disabled;
        } else if self.mode == ShieldsMode::Disabled {
            self.mode = ShieldsMode::Standard;
        }
    }

    /// Return the current `ShieldsMode` (`Standard`, `Aggressive`, `Disabled`).
    pub fn mode(&self) -> ShieldsMode {
        if !self.enabled {
            ShieldsMode::Disabled
        } else {
            self.mode
        }
    }

    /// Set the active `ShieldsMode`.
    pub fn set_mode(&mut self, mode: ShieldsMode) {
        self.mode = mode;
        self.enabled = mode != ShieldsMode::Disabled;
        self.block_third_party_subresources = mode == ShieldsMode::Aggressive;
    }

    /// Cycle through `Standard` -> `Aggressive` -> `Disabled` -> `Standard`.
    pub fn cycle_mode(&mut self) -> ShieldsMode {
        let next = self.mode().cycle_next();
        self.set_mode(next);
        next
    }

    /// Return whether strict third-party subresource blocking is enabled.
    pub fn blocks_third_party_subresources(&self) -> bool {
        self.block_third_party_subresources
    }

    /// Enable or disable strict third-party subresource blocking.
    pub fn set_block_third_party_subresources(&mut self, block: bool) {
        self.block_third_party_subresources = block;
    }

    /// Check whether Shields are enabled for a specific first-party `site_url_or_host`.
    pub fn is_enabled_for_site(&self, site_url_or_host: &str) -> bool {
        if !self.is_enabled() {
            return false;
        }
        let host = extract_normalized_host(site_url_or_host);
        if host.is_empty() {
            return true;
        }
        !self.disabled_sites.contains(&host)
            && !self.disabled_sites.contains(registrable_domain(&host))
    }

    /// Enable or disable Shields for a specific site host (`example.com`).
    pub fn set_site_enabled(&mut self, site_url_or_host: &str, enabled: bool) -> Option<String> {
        let host = extract_normalized_host(site_url_or_host);
        if host.is_empty() {
            return None;
        }
        if enabled {
            self.disabled_sites.remove(&host);
            self.disabled_sites.remove(registrable_domain(&host));
        } else {
            self.disabled_sites.insert(host.clone());
        }
        Some(host)
    }

    /// Toggle Shields Up/Down for `site_url_or_host`, returning `(host, new_enabled_state)`.
    pub fn toggle_site(&mut self, site_url_or_host: &str) -> Option<(String, bool)> {
        let currently_on = self.is_enabled_for_site(site_url_or_host);
        let host = self.set_site_enabled(site_url_or_host, !currently_on)?;
        Some((host, !currently_on))
    }

    /// List of sites where Shields are currently turned off (`Shields Down`).
    pub fn disabled_sites(&self) -> Vec<String> {
        let mut out: Vec<String> = self.disabled_sites.iter().cloned().collect();
        out.sort();
        out
    }

    /// Add a custom blocked domain at runtime and recompile the `adblock::Engine`.
    pub fn add_blocked_domain(&mut self, domain: &str) {
        let clean = domain.trim().trim_end_matches('.').to_ascii_lowercase();
        if !clean.is_empty() && !self.blocked_domains.contains(&clean) {
            self.blocked_domains.push(clean.clone());
            let _ = self.add_filter_rule(&format!("||{clean}^"));
        }
    }

    /// Add a single ABP / uBlock Origin network or cosmetic filter rule to Brave's `adblock::Engine`.
    pub fn add_filter_rule(&mut self, line: &str) -> bool {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('!') || trimmed.starts_with('[') {
            return false;
        }
        // Validate using adblock::filters::network::NetworkFilter or adblock::filters::cosmetic::CosmeticFilter
        let is_valid_cosmetic = adblock::filters::cosmetic::CosmeticFilter::parse(
            trimmed,
            false,
            PermissionMask::default(),
        )
        .is_ok();
        let is_valid_network =
            adblock::filters::network::NetworkFilter::parse(trimmed, true, ParseOptions::default())
                .is_ok();

        if !is_valid_cosmetic && !is_valid_network {
            return false;
        }

        if let Some(cls) = trimmed.strip_prefix("##.")
            && !cls.is_empty()
            && cls
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            self.custom_class_probes.push(cls.to_string());
        } else if let Some(id) = trimmed.strip_prefix("###")
            && !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            self.custom_id_probes.push(id.to_string());
        }

        self.all_rules.push(trimmed.to_string());
        self.custom_rules.push(trimmed.to_string());
        self.engine = Arc::new(build_brave_engine(&self.all_rules, &self.resources));
        true
    }

    /// Add a multi-line ABP / uBlock Origin filter list to Brave's `adblock::Engine`.
    pub fn add_filter_list(&mut self, list_text: &str) -> usize {
        let mut added = 0usize;
        for line in list_text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('!') || trimmed.starts_with('[') {
                continue;
            }
            let is_valid_cosmetic = adblock::filters::cosmetic::CosmeticFilter::parse(
                trimmed,
                false,
                PermissionMask::default(),
            )
            .is_ok();
            let is_valid_network = adblock::filters::network::NetworkFilter::parse(
                trimmed,
                true,
                ParseOptions::default(),
            )
            .is_ok();
            if is_valid_cosmetic || is_valid_network {
                self.all_rules.push(trimmed.to_string());
                self.custom_rules.push(trimmed.to_string());
                added += 1;
            }
        }
        if added > 0 {
            self.engine = Arc::new(build_brave_engine(&self.all_rules, &self.resources));
        }
        added
    }

    /// Custom ABP/uBO rules added during this session.
    pub fn custom_rules(&self) -> &[String] {
        &self.custom_rules
    }

    /// Total number of network filter rules loaded in the engine.
    pub fn network_rule_count(&self) -> usize {
        self.all_rules
            .iter()
            .filter(|r| !r.starts_with('!') && !r.contains("##") && !r.contains("#@#"))
            .count()
    }

    /// Total number of cosmetic element-hiding rules loaded in the engine.
    pub fn cosmetic_rule_count(&self) -> usize {
        self.all_rules
            .iter()
            .filter(|r| r.contains("##") || r.contains("#@#"))
            .count()
    }

    /// Total number of requests blocked by this shield instance.
    pub fn blocked_total(&self) -> u64 {
        self.blocked_total
    }

    /// Total number of tracking query parameters stripped by `$removeparam`.
    pub fn params_stripped_total(&self) -> u64 {
        self.params_stripped_total
    }

    /// Evaluate a network request directly through Brave's `adblock::Engine::check_network_request`.
    pub fn check_network_request(
        &mut self,
        url: &str,
        initiator_url: Option<&str>,
        request_type: RequestType,
    ) -> BlockerResult {
        if !self.is_enabled() {
            return BlockerResult::default();
        }
        if let Some(init) = initiator_url
            && !self.is_enabled_for_site(init)
        {
            return BlockerResult::default();
        }
        if request_type == RequestType::Document && !self.is_enabled_for_site(url) {
            return BlockerResult::default();
        }

        let source_url = initiator_url.unwrap_or(url);
        let Ok(brave_req) = BraveRequest::new(url, source_url, request_type.as_cpt_str(), "get")
        else {
            return BlockerResult::default();
        };

        // Call Brave's untouched `adblock::Engine::check_network_request_subset`!
        let brave_res: BraveBlockerResult = self
            .engine
            .check_network_request_subset(&brave_req, false, true);

        let redirect_resource = brave_res
            .redirect
            .as_deref()
            .and_then(decode_data_url_redirect);

        let matched_rule_str = brave_res
            .filter
            .as_ref()
            .and_then(|f| f.raw_line.as_deref());
        let exception_str = brave_res
            .exception
            .as_ref()
            .map(|e| e.raw_line.clone().unwrap_or_else(|| e.to_string()));

        if brave_res.should_block() {
            self.blocked_total += 1;
            let reason = Some(classify_brave_match_reason(
                matched_rule_str,
                &brave_req.hostname,
                &self.blocked_domains,
            ));
            return BlockerResult {
                matched: true,
                important: brave_res.important,
                reason,
                exception: exception_str,
                redirect_resource,
                rewritten_url: brave_res.rewritten_url,
            };
        }

        // If an `@@` exception rule explicitly matched in Brave's engine, honor it!
        if exception_str.is_some() {
            if brave_res.rewritten_url.is_some() {
                self.params_stripped_total += 1;
            }
            return BlockerResult {
                matched: false,
                important: false,
                reason: None,
                exception: exception_str,
                redirect_resource: None,
                rewritten_url: brave_res.rewritten_url,
            };
        }

        // Additional check for telemetry path substrings and strict third-party mode
        if let Some(fallback_reason) = self.check_supplementary_rules(&brave_req, initiator_url) {
            self.blocked_total += 1;
            return BlockerResult {
                matched: true,
                important: false,
                reason: Some(fallback_reason),
                exception: None,
                redirect_resource,
                rewritten_url: None,
            };
        }

        if brave_res.rewritten_url.is_some() {
            self.params_stripped_total += 1;
        }

        BlockerResult {
            matched: false,
            important: false,
            reason: None,
            exception: None,
            redirect_resource: None,
            rewritten_url: brave_res.rewritten_url,
        }
    }

    fn check_supplementary_rules(
        &self,
        req: &BraveRequest,
        initiator_url: Option<&str>,
    ) -> Option<ShieldBlockReason> {
        let lower_url = req.url.to_ascii_lowercase();
        for pat in &self.telemetry_patterns {
            if lower_url.contains(pat.as_str()) {
                return Some(ShieldBlockReason::TelemetryPath(pat.clone()));
            }
        }
        if (self.block_third_party_subresources || self.mode == ShieldsMode::Aggressive)
            && req.is_third_party
            && req.request_type != adblock::request::RequestType::Document
        {
            let init_host = initiator_url
                .and_then(|u| parse_url(u).ok())
                .map(|p| p.host.to_ascii_lowercase())
                .unwrap_or_default();
            return Some(ShieldBlockReason::ThirdPartySubresource {
                initiator_host: init_host,
                target_host: req.hostname.clone(),
            });
        }
        None
    }

    /// Clean tracking query parameters (`utm_*`, `fbclid`, `gclid`, `msclkid`, `$removeparam`)
    /// from `url` using Brave's `adblock::Engine::check_network_request` (`rewritten_url`).
    pub fn clean_tracking_params(&mut self, url: &str, initiator_url: Option<&str>) -> String {
        if !self.is_enabled() || !self.is_enabled_for_site(url) {
            return url.to_string();
        }
        let source_url = initiator_url.unwrap_or(url);
        let Ok(brave_req) = BraveRequest::new(url, source_url, "document", "get") else {
            return url.to_string();
        };
        let brave_res = self.engine.check_network_request(&brave_req);
        if let Some(rewritten) = brave_res.rewritten_url {
            self.params_stripped_total += 1;
            rewritten
        } else {
            url.to_string()
        }
    }

    /// Evaluate whether `url` (optionally initiated by `initiator_url`) should be blocked.
    /// If blocked, increments `blocked_total` and returns `Some(ShieldBlockReason)`.
    pub fn check_url(
        &mut self,
        url: &str,
        initiator_url: Option<&str>,
    ) -> Option<ShieldBlockReason> {
        let rt = if initiator_url.is_some() {
            RequestType::Other
        } else {
            RequestType::Document
        };
        self.check_network_request(url, initiator_url, rt).reason
    }

    /// Alias for [`Self::check_url`] that evaluates `url` and increments `blocked_total`.
    pub fn check_and_count(
        &mut self,
        url: &str,
        initiator_url: Option<&str>,
    ) -> Option<ShieldBlockReason> {
        self.check_url(url, initiator_url)
    }

    /// Inspect `url` without mutating counters, returning `Some(ShieldBlockReason)` if blocked.
    pub fn inspect_url(&self, url: &str, initiator_url: Option<&str>) -> Option<ShieldBlockReason> {
        if !self.is_enabled() {
            return None;
        }
        if let Some(init) = initiator_url
            && !self.is_enabled_for_site(init)
        {
            return None;
        }
        let rt = if initiator_url.is_some() {
            RequestType::Other
        } else {
            RequestType::Document
        };
        if rt == RequestType::Document && !self.is_enabled_for_site(url) {
            return None;
        }
        let source_url = initiator_url.unwrap_or(url);
        let brave_req = BraveRequest::new(url, source_url, rt.as_cpt_str(), "get").ok()?;
        let brave_res = self
            .engine
            .check_network_request_subset(&brave_req, false, true);
        if brave_res.should_block() {
            let matched_rule_str = brave_res
                .filter
                .as_ref()
                .and_then(|f| f.raw_line.as_deref());
            return Some(classify_brave_match_reason(
                matched_rule_str,
                &brave_req.hostname,
                &self.blocked_domains,
            ));
        }
        if brave_res.exception.is_some() {
            return None;
        }
        self.check_supplementary_rules(&brave_req, initiator_url)
    }

    /// Increment the blocked request counter by 1.
    pub fn record_block(&mut self) {
        self.blocked_total += 1;
    }

    /// Compute cosmetic element-hiding selectors and `:style(...)` rules for `page_url`
    /// directly via Brave's `adblock::Engine::url_cosmetic_resources` and `hidden_class_id_selectors`.
    pub fn cosmetic_resources_for_url(&self, page_url: &str) -> CosmeticResources {
        if !self.is_enabled() || !self.is_enabled_for_site(page_url) {
            return CosmeticResources::default();
        }
        let Ok(parsed) = parse_url(page_url) else {
            return CosmeticResources::default();
        };
        if parsed.scheme != "http" && parsed.scheme != "https" {
            return CosmeticResources::default();
        }

        // 1. Query Brave's `adblock::Engine::url_cosmetic_resources`
        let url_res: BraveUrlSpecificResources = self.engine.url_cosmetic_resources(page_url);

        let mut hide_selectors: Vec<String> = url_res.hide_selectors.into_iter().collect();
        let mut style_rules: Vec<(String, String)> = Vec::new();

        // 2. Extract pure CSS `:style(...)` rules from `procedural_actions`
        for raw_json in &url_res.procedural_actions {
            if let Some((sel, style)) = extract_css_from_procedural_json(raw_json) {
                if style == "display: none !important" {
                    if !hide_selectors.contains(&sel) {
                        hide_selectors.push(sel);
                    }
                } else {
                    style_rules.push((sel, style));
                }
            }
        }

        // 3. Query Brave's `adblock::Engine::hidden_class_id_selectors` for generic class/id rules
        if !url_res.generichide {
            let classes_iter = DEFAULT_GENERIC_AD_CLASSES
                .iter()
                .copied()
                .chain(self.custom_class_probes.iter().map(String::as_str));
            let ids_iter = DEFAULT_GENERIC_AD_IDS
                .iter()
                .copied()
                .chain(self.custom_id_probes.iter().map(String::as_str));
            let generic_selectors =
                self.engine
                    .hidden_class_id_selectors(classes_iter, ids_iter, &url_res.exceptions);
            for sel in generic_selectors {
                if !hide_selectors.contains(&sel) {
                    hide_selectors.push(sel);
                }
            }
        }

        hide_selectors.sort();

        CosmeticResources {
            hide_selectors,
            style_rules,
            exceptions: url_res.exceptions,
            injected_script: url_res.injected_script,
            generichide: url_res.generichide,
        }
    }

    /// Generate a CSS stylesheet string (`selector { display: none !important; }`) for `page_url`.
    pub fn cosmetic_stylesheet_for_url(&self, page_url: &str) -> String {
        self.cosmetic_resources_for_url(page_url).to_stylesheet()
    }
}

/// Parse a JSON-serialized `adblock::cosmetic_filter_cache::ProceduralOrActionFilter`
/// and extract `(selector, style)` if it represents a pure CSS rule.
fn extract_css_from_procedural_json(raw_json: &str) -> Option<(String, String)> {
    let val = anvil_json::parse_json(raw_json).ok()?;
    let selectors = val.get("selector")?.as_array()?;
    if selectors.len() != 1 {
        return None;
    }
    let op = &selectors[0];
    if op.get("type")?.as_str()? != "css-selector" {
        return None;
    }
    let css_sel = op.get("arg")?.as_str()?.to_string();
    match val.get("action") {
        None => Some((css_sel, "display: none !important".to_string())),
        Some(act) if act.is_null() => Some((css_sel, "display: none !important".to_string())),
        Some(act) => {
            if act.get("type")?.as_str()? == "style" {
                let style_str = act.get("arg")?.as_str()?.to_string();
                Some((css_sel, style_str))
            } else {
                None
            }
        }
    }
}

fn classify_brave_match_reason(
    filter_rule: Option<&str>,
    req_hostname: &str,
    blocked_domains: &[String],
) -> ShieldBlockReason {
    for domain in blocked_domains {
        if req_hostname == *domain || req_hostname.ends_with(&format!(".{domain}")) {
            return ShieldBlockReason::TrackerDomain(domain.clone());
        }
    }
    if let Some(rule) = filter_rule {
        let lower = rule.to_ascii_lowercase();
        for pat in DEFAULT_TELEMETRY_PATTERNS {
            if lower.contains(pat) {
                return ShieldBlockReason::TelemetryPath((*pat).to_string());
            }
        }
        if let Some(rest) = rule.strip_prefix("||") {
            let end = rest.find(['^', '/', '*', '$']).unwrap_or(rest.len());
            let host = &rest[..end];
            if !host.is_empty() && !host.contains('*') {
                return ShieldBlockReason::TrackerDomain(host.to_ascii_lowercase());
            }
        }
        return ShieldBlockReason::FilterRule(rule.to_string());
    }
    ShieldBlockReason::TrackerDomain(req_hostname.to_string())
}

/// Decode a `data:<mime>;base64,<payload>` redirect string returned by `adblock::Engine`.
fn decode_data_url_redirect(data_url: &str) -> Option<(String, String)> {
    let rest = data_url.strip_prefix("data:")?;
    let (mime, b64) = rest.split_once(";base64,")?;
    let bytes = base64_decode(b64)?;
    let text = String::from_utf8(bytes).ok()?;
    Some((mime.to_string(), text))
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for b in input.bytes() {
        if b == b'=' || b.is_ascii_whitespace() {
            continue;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32;
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

fn extract_normalized_host(url_or_host: &str) -> String {
    let trimmed = url_or_host.trim();
    if trimmed.is_empty() || trimmed.starts_with("anvil://") || trimmed.starts_with("about:") {
        return String::new();
    }
    if let Ok(parsed) = parse_url(trimmed) {
        return parsed.host.trim_end_matches('.').to_ascii_lowercase();
    }
    let candidate = trimmed
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if candidate.contains('.') {
        candidate
    } else {
        String::new()
    }
}

/// Extract a rough registrable domain (`eTLD+1` approximation, last two labels) for cross-site checks.
fn registrable_domain(host: &str) -> &str {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() <= 2 {
        return host;
    }
    let last_two_len = parts[parts.len() - 2].len() + 1 + parts[parts.len() - 1].len();
    &host[host.len() - last_two_len..]
}

static DEFAULT_SHIELD: OnceLock<Mutex<NetworkShield>> = OnceLock::new();

fn default_shield_mutex() -> &'static Mutex<NetworkShield> {
    DEFAULT_SHIELD.get_or_init(|| Mutex::new(NetworkShield::new()))
}

/// Synchronize the process-wide default shield with `shield` so worker fetches, stylesheet/image
/// loads, and `anvil_net::fetch` honor the active session's Shields mode, site exceptions, and custom rules.
pub fn sync_default_shield(shield: &NetworkShield) {
    let mut guard = default_shield_mutex()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *guard = shield.clone();
}

/// Stateless convenience check using the default [`NetworkShield`] ruleset.
pub fn is_url_blocked_by_default_shield(
    url: &str,
    initiator_url: Option<&str>,
) -> Option<ShieldBlockReason> {
    let mut guard = default_shield_mutex()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    guard.check_url(url, initiator_url)
}

/// Check a typed subresource or navigation request against the process-wide default [`NetworkShield`],
/// returning a full [`BlockerResult`] (including `$redirect` surrogates and `$removeparam` URLs).
pub fn check_default_shield_request(
    url: &str,
    initiator_url: Option<&str>,
    request_type: RequestType,
) -> BlockerResult {
    let mut guard = default_shield_mutex()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    guard.check_network_request(url, initiator_url, request_type)
}

/// Clean tracking query parameters from `url` using the process-wide default [`NetworkShield`].
pub fn clean_url_with_default_shield(url: &str, initiator_url: Option<&str>) -> String {
    let mut guard = default_shield_mutex()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    guard.clean_tracking_params(url, initiator_url)
}

/// Generate the cosmetic ad-hiding CSS stylesheet for `page_url` from the process-wide default [`NetworkShield`].
pub fn cosmetic_css_from_default_shield(page_url: &str) -> String {
    let guard = default_shield_mutex()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    guard.cosmetic_stylesheet_for_url(page_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_known_tracker_domains_and_subdomains() {
        let mut shield = NetworkShield::new();
        let res = shield.check_url(
            "https://stats.g.doubleclick.net/r/collect?id=123",
            Some("https://example.com/index.html"),
        );
        assert!(matches!(res, Some(ShieldBlockReason::TrackerDomain(_))));
        assert_eq!(shield.blocked_total(), 1);
    }

    #[test]
    fn allows_clean_first_party_and_cdn_requests() {
        let mut shield = NetworkShield::new();
        assert!(
            shield
                .check_url(
                    "https://example.com/assets/app.css",
                    Some("https://example.com/")
                )
                .is_none()
        );
        assert!(
            shield
                .check_url(
                    "https://sub.example.com/img/logo.png",
                    Some("https://www.example.com/")
                )
                .is_none()
        );
        assert_eq!(shield.blocked_total(), 0);
    }

    #[test]
    fn blocks_third_party_telemetry_path_patterns() {
        let mut shield = NetworkShield::new();
        let res = shield.check_url(
            "https://cdn.other-site.org/scripts/adsbygoogle.js",
            Some("https://news.example.com/article"),
        );
        assert!(matches!(res, Some(ShieldBlockReason::TelemetryPath(_))));
    }

    #[test]
    fn optional_strict_third_party_blocking_enforces_registrable_domain() {
        let mut shield = NetworkShield::new();
        shield.set_block_third_party_subresources(true);
        assert!(
            shield
                .check_url(
                    "https://static.example.com/bundle.js",
                    Some("https://app.example.com/index.html")
                )
                .is_none()
        );
        let cross = shield.check_url(
            "https://cdn.unrelated.org/bundle.js",
            Some("https://app.example.com/index.html"),
        );
        assert!(matches!(
            cross,
            Some(ShieldBlockReason::ThirdPartySubresource { .. })
        ));
    }

    #[test]
    fn runs_unmodified_brave_adblock_engine_network_filters_and_exceptions() {
        let mut shield = NetworkShield::new();
        assert!(shield.add_filter_rule("||ads.badtracker.org^$third-party,script"));
        assert!(
            shield.add_filter_rule(
                "@@||ads.badtracker.org/allow.js$script,domain=trusted.example.com"
            )
        );
        assert!(shield.add_filter_rule("||forceblock.badtracker.org^$important"));
        assert!(shield.add_filter_rule("@@||forceblock.badtracker.org^"));

        // 1. Third-party script to ads.badtracker.org is blocked by adblock::Engine
        let r1 = shield.check_network_request(
            "https://sub.ads.badtracker.org/banner.js",
            Some("https://news.example.com/"),
            RequestType::Script,
        );
        assert!(r1.matched);

        // 2. First-party request to ads.badtracker.org is allowed because of $third-party
        let r2 = shield.check_network_request(
            "https://ads.badtracker.org/banner.js",
            Some("https://ads.badtracker.org/"),
            RequestType::Script,
        );
        assert!(!r2.matched);

        // 3. Image request to ads.badtracker.org is allowed because of $script
        let r3 = shield.check_network_request(
            "https://ads.badtracker.org/banner.png",
            Some("https://news.example.com/"),
            RequestType::Image,
        );
        assert!(!r3.matched);

        // 4. Exception rule on trusted.example.com unblocks allow.js
        let r4 = shield.check_network_request(
            "https://ads.badtracker.org/allow.js",
            Some("https://trusted.example.com/page"),
            RequestType::Script,
        );
        assert!(!r4.matched);
        assert!(r4.exception.is_some());

        // 5. $important rule overrides @@ exception rule
        let r5 = shield.check_network_request(
            "https://forceblock.badtracker.org/tracker.js",
            Some("https://trusted.example.com/page"),
            RequestType::Script,
        );
        assert!(r5.matched);
        assert!(r5.important);
    }

    #[test]
    fn redirect_surrogates_and_removeparam_query_stripping_work_via_brave_engine() {
        let mut shield = NetworkShield::new();

        // Redirect surrogate for google-analytics.com/analytics.js via adblock::Engine
        let res = shield.check_network_request(
            "https://www.google-analytics.com/analytics.js",
            Some("https://shop.example.com/"),
            RequestType::Script,
        );
        assert!(res.matched);
        let (mime, body) = res.redirect_resource.expect("should return GA surrogate");
        assert_eq!(mime, "application/javascript");
        assert!(body.contains("GoogleAnalyticsObject"));

        // Tracking query parameter stripping ($removeparam) via adblock::Engine
        let dirty = "https://example.com/product?id=42&utm_source=newsletter&fbclid=IwAR123&gclid=abc&sort=price#details";
        let cleaned = shield.clean_tracking_params(dirty, None);
        assert_eq!(
            cleaned,
            "https://example.com/product?id=42&sort=price#details"
        );
        assert!(shield.params_stripped_total() >= 1);
    }

    #[test]
    fn cosmetic_filter_engine_generates_css_and_honors_domain_exceptions() {
        let mut shield = NetworkShield::new();
        assert!(shield.add_filter_rule("shop.example.com##.promo-popup"));
        assert!(shield.add_filter_rule("shop.example.com#@#.ad-banner"));
        assert!(shield.add_filter_rule("shop.example.com##body:style(overflow: auto !important)"));

        let res = shield.cosmetic_resources_for_url("https://shop.example.com/deals");
        // `.promo-popup` is included for shop.example.com
        assert!(res.hide_selectors.iter().any(|s| s == ".promo-popup"));
        // `.ad-banner` was excepted via `#@#` on shop.example.com
        assert!(!res.hide_selectors.iter().any(|s| s == ".ad-banner"));
        // Generic `.adsbox` is still hidden
        assert!(res.hide_selectors.iter().any(|s| s == ".adsbox"));
        let css = res.to_stylesheet();
        assert!(css.contains(".promo-popup { display: none !important; }"));
        assert!(css.contains("body { overflow: auto !important; }"));
    }

    #[test]
    fn per_site_shields_toggle_and_modes_control_blocking() {
        let mut shield = NetworkShield::new();
        let (host, enabled) = shield
            .toggle_site("https://allowlisted.example.org/news")
            .expect("valid host");
        assert_eq!(host, "allowlisted.example.org");
        assert!(!enabled);
        assert!(!shield.is_enabled_for_site("https://allowlisted.example.org/other"));

        // Requests initiated by allowlisted.example.org bypass Shields
        let r = shield.check_network_request(
            "https://stats.g.doubleclick.net/r/collect",
            Some("https://allowlisted.example.org/news"),
            RequestType::Script,
        );
        assert!(!r.matched);
        assert_eq!(
            shield.cosmetic_stylesheet_for_url("https://allowlisted.example.org/news"),
            ""
        );

        // Toggle back on
        let (_, enabled_again) = shield
            .toggle_site("https://allowlisted.example.org/news")
            .unwrap();
        assert!(enabled_again);
        assert!(
            shield
                .check_network_request(
                    "https://stats.g.doubleclick.net/r/collect",
                    Some("https://allowlisted.example.org/news"),
                    RequestType::Script,
                )
                .matched
        );
    }
}
