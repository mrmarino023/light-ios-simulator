//! Boot-time job profiles — passed to `simctl boot --disabledJob=…` (after UDID).

use std::collections::HashSet;
use std::sync::OnceLock;

/// Feature flags that **keep** certain launchd jobs enabled (not disabled at boot).
#[derive(Debug, Clone)]
pub struct FeatureRequirements {
    pub push: bool,
    pub storekit: bool,
    pub camera: bool,
    pub icloud: bool,
    /// Keep WidgetKit / chronod. Default on — slim boot otherwise blanks home widgets.
    pub widgets: bool,
}

impl Default for FeatureRequirements {
    fn default() -> Self {
        Self {
            push: false,
            storekit: false,
            camera: false,
            icloud: false,
            widgets: true,
        }
    }
}

impl FeatureRequirements {
    pub fn parse_csv(s: &str) -> Self {
        let mut req = Self::default();
        for part in s.split(',') {
            match part.trim().to_ascii_lowercase().as_str() {
                "push" | "aps" => req.push = true,
                "storekit" | "store" | "iap" => req.storekit = true,
                "camera" => req.camera = true,
                "icloud" => req.icloud = true,
                "nowidgets" | "slim-widgets" => req.widgets = false,
                "widgets" => req.widgets = true,
                _ => {}
            }
        }
        req
    }
}

static SLIM_LABELS: OnceLock<Vec<&'static str>> = OnceLock::new();

/// Full SimSlim-compatible managed label set (171 services on iOS 18).
pub fn slim_labels() -> &'static [&'static str] {
    SLIM_LABELS.get_or_init(|| {
        include_str!("../labels.txt")
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect()
    })
}

/// Jobs that must stay **enabled** when a feature is required.
fn protected_jobs(req: &FeatureRequirements) -> HashSet<&'static str> {
    let mut keep = HashSet::new();
    if req.push {
        keep.insert("com.apple.apsd");
    }
    if req.storekit {
        for j in [
            "com.apple.storekitd",
            "com.apple.itunesstored",
            "com.apple.passd",
            "com.apple.financed",
            "com.apple.amsaccountsd",
            "com.apple.amsengagementd",
            "com.apple.amsondevicestoraged",
            "com.apple.appstored",
            "com.apple.appstorecomponentsd",
            "com.apple.videosubscriptionsd",
            "com.apple.assetsubscriptiond",
        ] {
            keep.insert(j);
        }
    }
    if req.widgets {
        for j in [
            "com.apple.chronod",
            "com.apple.PosterBoard",
            "com.apple.contacts.postersyncd",
        ] {
            keep.insert(j);
        }
    }
    if req.icloud {
        for j in [
            "com.apple.cloudd",
            "com.apple.akd",
            "com.apple.bird",
            "com.apple.cloudphotod",
            "com.apple.appleaccountd",
            "com.apple.amsaccountsd",
        ] {
            keep.insert(j);
        }
    }
    keep
}

/// Resolved job list for boot `--disabledJob` (full slim set minus protected features).
pub fn resolve_disabled_jobs(req: &FeatureRequirements) -> Vec<String> {
    let keep = protected_jobs(req);
    slim_labels()
        .iter()
        .filter(|j| !keep.contains(*j))
        .map(|s| (*s).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storekit_keeps_store_jobs() {
        let req = FeatureRequirements {
            storekit: true,
            ..Default::default()
        };
        let jobs = resolve_disabled_jobs(&req);
        assert!(!jobs.iter().any(|j| j == "com.apple.storekitd"));
        assert!(jobs.iter().any(|j| j == "com.apple.searchd"));
    }

    #[test]
    fn widgets_default_keeps_chronod() {
        let jobs = resolve_disabled_jobs(&FeatureRequirements::default());
        assert!(!jobs.iter().any(|j| j == "com.apple.chronod"));
        let slim = FeatureRequirements {
            widgets: false,
            ..Default::default()
        };
        let jobs = resolve_disabled_jobs(&slim);
        assert!(jobs.iter().any(|j| j == "com.apple.chronod"));
    }

    #[test]
    fn slim_label_count() {
        assert!(slim_labels().len() >= 150);
    }
}
