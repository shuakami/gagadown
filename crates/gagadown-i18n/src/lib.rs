//! Fluent catalogs shared by presentation layers. The download core stays locale-neutral.
//! Static labels are resolved once and borrowed by index without locks or formatting.
pub use fluent_bundle::FluentArgs;
use fluent_bundle::{FluentBundle, FluentResource};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    Chinese,
    English,
}

impl Language {
    pub fn resolve(preference: &str, system: Option<&str>) -> Self {
        let locale = if preference == "system" { system.unwrap_or("zh-CN") } else { preference };
        if locale.split(['-', '_']).next().is_some_and(|s| s.eq_ignore_ascii_case("en")) {
            Self::English
        } else {
            Self::Chinese
        }
    }

    pub fn from_preference(preference: &str) -> Self {
        Self::resolve(preference, sys_locale::get_locale().as_deref())
    }

    pub fn tag(self) -> &'static str {
        match self { Self::Chinese => "zh-CN", Self::English => "en" }
    }
}

macro_rules! labels {
    ($($variant:ident => $key:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug)]
        #[repr(usize)]
        pub enum Label { $($variant),+ }
        const LABEL_KEYS: &[&str] = &[$($key),+];
    };
}

labels! {
    Language => "language",
    SystemLanguage => "system-language",
    Pause => "pause",
    Resume => "resume",
    Cancel => "cancel",
    Delete => "delete",
    Settings => "settings",
    Downloads => "downloads",
    Completed => "completed",
    Trash => "trash",
    DirectThenProxy => "direct-then-proxy",
    DirectOnly => "direct-only",
    ProxyThenDirect => "proxy-then-direct",
    ProxyOnly => "proxy-only",
    General => "general",
    Download => "download",
    Cache => "cache",
    BrowserExtension => "browser-extension",
    LaunchAtLogin => "launch-at-login",
    SilentStartup => "silent-startup",
    SilentStartupHelp => "silent-startup-help",
    DownloadDirectory => "download-directory",
    CacheDirectory => "cache-directory",
    Change => "change",
    Default => "default",
    ConcurrentTasks => "concurrent-tasks",
    ConcurrentTasksHelp => "concurrent-tasks-help",
    InitialConnections => "initial-connections",
    InitialConnectionsHelp => "initial-connections-help",
    TaskConnections => "task-connections",
    TaskConnectionsHelp => "task-connections-help",
    TotalConnections => "total-connections",
    TotalConnectionsHelp => "total-connections-help",
    MinimumSegment => "minimum-segment",
    MinimumSegmentHelp => "minimum-segment-help",
    SpeedLimit => "speed-limit",
    UnlimitedHelp => "unlimited-help",
    SegmentRetries => "segment-retries",
    TaskRetries => "task-retries",
    Network => "network",
    ProxyMode => "proxy-mode",
    ProxyAddresses => "proxy-addresses",
    OnePerLine => "one-per-line",
    DetectProxies => "detect-proxies",
    DetectProxiesHelp => "detect-proxies-help",
    SystemProxy => "system-proxy",
    DirectWait => "direct-wait",
    DirectWaitHelp => "direct-wait-help",
    DetectedProxies => "detected-proxies",
    None => "none",
    Browser => "browser",
    TakeoverMinimum => "takeover-minimum",
    TakeoverMinimumHelp => "takeover-minimum-help",
    LocalPort => "local-port",
    RestartRequired => "restart-required",
    Cleanup => "cleanup",
    TrashRetention => "trash-retention",
    OrphanCleanup => "orphan-cleanup",
    NeverCleanupHelp => "never-cleanup-help",
    SyncComplete => "sync-complete",
    SyncCompleteHelp => "sync-complete-help",
    DaysSuffix => "days-suffix",
    LinkPlaceholder => "link-placeholder",
    CliUrl => "cli-url",
    CliDirectory => "cli-directory",
    CliMaxConnections => "cli-max-connections",
    CliInitial => "cli-initial",
    CliProxy => "cli-proxy",
    CliDirectOnly => "cli-direct-only",
    CliSha256 => "cli-sha256",
    CliPort => "cli-port",
    CliAbout => "cli-about",
    CliGet => "cli-get",
    CliServe => "cli-serve",
    CliProxies => "cli-proxies",
    CliDataDirectory => "cli-data-directory",
    CliLanguage => "cli-language",
    CliConnections => "cli-connections",
    CliSplits => "cli-splits",
    CliFailed => "cli-failed",
    Refresh => "refresh",
    Downloading => "downloading",
    OrphanFiles => "orphan-files",
    DiskFree => "disk-free",
    CleanOrphans => "clean-orphans",
    CleanTrashCache => "clean-trash-cache",
    Orphan => "orphan",
    ExtensionIntro => "extension-intro",
    BrowserNotFound => "browser-not-found",
    OpenExtensionFolder => "open-extension-folder",
    Connected => "connected",
    Disconnected => "disconnected",
    Reinstall => "reinstall",
    InstallExtension => "install-extension",
    EdgeDeveloperMode => "edge-developer-mode",
    EdgeLoadUnpacked => "edge-load-unpacked",
    DeveloperMode => "developer-mode",
    LoadUnpacked => "load-unpacked",
    InstallationSteps => "installation-steps",
    InstallExplanation => "install-explanation",
    ExtensionSelectFolder => "extension-select-folder",
    Copy => "copy",
    OpenDirectory => "open-directory",
    ExtensionConnected => "extension-connected",
    ExtensionWaiting => "extension-waiting",
    PathCopied => "path-copied",
    AddressCopied => "address-copied",
    LinkCopied => "link-copied",
    ExtensionPathCopied => "extension-path-copied",
    ShowWindow => "show-window",
    Exit => "exit",
    ErrorReport => "error-report",
    CopyReport => "copy-report",
    Close => "close",
    DeleteTask => "delete-task",
    KeepFiles => "keep-files",
    TrashFiles => "trash-files",
    DeleteFiles => "delete-files",
    UnknownSize => "unknown-size",
    BrowserContinues => "browser-continues",
    Connecting => "connecting",
    DownloadComplete => "download-complete",
    OpenFolder => "open-folder",
    Open => "open",
    Retry => "retry",
    About => "about",
    OpenDataDirectory => "open-data-directory",
    DataDirectoryHelp => "data-directory-help",
}

pub struct Catalog {
    language: Language,
    bundle: FluentBundle<FluentResource>,
    labels: Vec<String>,
}

impl Catalog {
    /// Catalog errors are returned, never converted to a release-mode abort.
    pub fn new(language: Language) -> Result<Self, String> {
        let source = match language {
            Language::Chinese => include_str!("../locales/zh-CN.ftl"),
            Language::English => include_str!("../locales/en.ftl"),
        };
        let resource = FluentResource::try_new(source.to_owned())
            .map_err(|(_, errors)| format!("Invalid Fluent resource: {errors:?}"))?;
        let id = language.tag().parse().map_err(|e| format!("Invalid locale: {e}"))?;
        let mut bundle = FluentBundle::new(vec![id]);
        bundle.add_resource(resource).map_err(|e| format!("Duplicate Fluent resource: {e:?}"))?;
        let mut catalog = Self { language, bundle, labels: Vec::with_capacity(LABEL_KEYS.len()) };
        for key in LABEL_KEYS {
            catalog.labels.push(catalog.format(key, None)?);
        }
        Ok(catalog)
    }

    pub fn language(&self) -> Language { self.language }

    #[inline]
    pub fn text(&self, label: Label) -> &str { &self.labels[label as usize] }

    /// Replace the complete snapshot only after the new catalog validates successfully.
    pub fn switch(&mut self, language: Language) -> Result<(), String> {
        if self.language != language { *self = Self::new(language)?; }
        Ok(())
    }

    pub fn format(&self, key: &str, args: Option<&FluentArgs<'_>>) -> Result<String, String> {
        let pattern = self.bundle.get_message(key).and_then(|m| m.value())
            .ok_or_else(|| format!("Missing Fluent message: {key}"))?;
        let mut errors = Vec::new();
        let value = self.bundle.format_pattern(pattern, args, &mut errors);
        if errors.is_empty() { Ok(value.into_owned()) }
        else { Err(format!("Fluent message {key}: {errors:?}")) }
    }

    /// Format a presentation message with string parameters; errors remain visible.
    pub fn message(&self, key: &str, values: &[(&str, &str)]) -> String {
        let mut args = FluentArgs::new();
        for &(name, value) in values { args.set(name, value); }
        self.format(key, Some(&args)).unwrap_or_else(|error| error)
    }

    pub fn completed_downloads(&self, count: u32, size: &str) -> Result<String, String> {
        let mut args = FluentArgs::new();
        args.set("count", count);
        args.set("size", size);
        self.format("completed-downloads", Some(&args))
    }

    pub fn task_count(&self, count: u32) -> Result<String, String> {
        let mut args = FluentArgs::new();
        args.set("count", count);
        self.format("task-count", Some(&args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_labels_switch_without_changing_keys() {
        let mut catalog = Catalog::new(Language::Chinese).unwrap();
        assert_eq!(catalog.text(Label::Pause), "暂停");
        let address = catalog.text(Label::Pause).as_ptr();
        assert_eq!(address, catalog.text(Label::Pause).as_ptr());
        catalog.switch(Language::English).unwrap();
        assert_eq!(catalog.text(Label::Pause), "Pause");
        for key in LABEL_KEYS { assert!(!catalog.format(key, None).unwrap().is_empty()); }
    }

    #[test]
    fn missing_keys_and_arguments_are_errors() {
        for language in [Language::Chinese, Language::English] {
            let catalog = Catalog::new(language).unwrap();
            assert!(catalog.format("does-not-exist", None).is_err());
            assert!(catalog.format("task-count", None).is_err());
        }
    }

    #[test]
    fn english_plurals() {
        let catalog = Catalog::new(Language::English).unwrap();
        for (count, expected) in [(0, "0 tasks"), (1, "1 task"), (2, "2 tasks")] {
            // Fluent isolates interpolated text to preserve bidirectional correctness.
            let text = catalog.task_count(count).unwrap().replace(['\u{2068}', '\u{2069}'], "");
            assert_eq!(text, expected);
        }
    }

    #[test]
    fn explicit_default_and_system_language() {
        assert_eq!(Language::default(), Language::Chinese);
        assert_eq!(Language::resolve("zh-CN", Some("en-US")), Language::Chinese);
        assert_eq!(Language::resolve("en", Some("zh-CN")), Language::English);
        assert_eq!(Language::resolve("system", Some("en_US")), Language::English);
        assert_eq!(Language::resolve("system", Some("de-DE")), Language::Chinese);
        assert_eq!(Language::resolve("system", None), Language::Chinese);
    }
}
