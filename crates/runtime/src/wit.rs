use std::collections::{HashMap, HashSet};

/// A collection of [`WitInterface`]s that represent a world in a WIT file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WitWorld {
    pub imports: HashSet<WitInterface>,
    pub exports: HashSet<WitInterface>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WitInterface {
    pub namespace: String,
    pub package: String,
    // TODO: it would be best for me to impl PartialEq here to
    // ensure interfaces can be compared
    pub interfaces: Vec<String>,
    pub version: Option<semver::Version>,
    pub config: HashMap<String, String>,
}

impl std::hash::Hash for WitInterface {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.namespace.hash(state);
        self.package.hash(state);
        self.interfaces.hash(state);
        self.version.hash(state);
        for (k, v) in &self.config {
            k.hash(state);
            v.hash(state);
        }
    }
}

impl From<&str> for WitInterface {
    fn from(s: &str) -> Self {
        // Expected format: namespace:package/interface@version
        // Also supports for convenience: namespace:package/interface,interface2,interface3@version
        // interface and version are optional

        let (main, version) = match s.split_once('@') {
            Some((m, v)) => (m, Some(v)),
            None => (s, None),
        };
        let (namespace_package, interface) = match main.split_once('/') {
            Some((np, iface)) => (np, Some(iface)),
            None => (main, None),
        };
        let (namespace, package) = match namespace_package.split_once(':') {
            Some((ns, pkg)) => (ns, pkg),
            None => ("", namespace_package),
        };
        let interfaces = match interface {
            Some(iface) => iface
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            None => vec![],
        };
        let version = version.and_then(|v| semver::Version::parse(v).ok());

        WitInterface {
            namespace: namespace.to_string(),
            package: package.to_string(),
            interfaces,
            version,
            config: HashMap::new(),
        }
    }
}
