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
    pub interfaces: Vec<String>,
    pub version: Option<semver::Version>,
    // consideration: Config here is overrides
    // consideration: Is this a reference or values?
    // e.g. redis_url: 127.0.0.1:6379
    // or   profile: work_queue, then the host fetches
    pub config: HashMap<String, String>,
    // For wasi:http/incoming, Host: <host-to-respond-to>, not name.namespace
    // Need to load balance internally to hit components, round robin between different components that register the same host
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

// impl WitInterface {
//     fn merge(self, other: Self) -> (Self, Option<Self>) {
//         if self.namespace == other.namespace
//             && self.package == other.package
//             && self.version == other.version
//         {
//             // Merge interfaces (deduped)
//             let mut merged_interfaces = self.interfaces.clone();
//             for iface in other.interfaces {
//                 if !merged_interfaces.contains(&iface) {
//                     merged_interfaces.push(iface);
//                 }
//             }
//             // Always merge both config maps, preferring self's values
//             let mut merged_config = self.config.clone();
//             for (k, v) in other.config.iter() {
//                 merged_config.entry(k.clone()).or_insert_with(|| v.clone());
//             }

//             let merged = WitInterface {
//                 namespace: self.namespace,
//                 package: self.package,
//                 interfaces: merged_interfaces,
//                 version: self.version.clone(),
//                 config: merged_config,
//             };
//             (merged, None)
//         } else {
//             (
//                 WitInterface {
//                     namespace: self.namespace,
//                     package: self.package,
//                     interfaces: self.interfaces,
//                     version: self.version.clone(),
//                     config: self.config.clone(),
//                 },
//                 Some(WitInterface {
//                     namespace: other.namespace,
//                     package: other.package,
//                     interfaces: other.interfaces,
//                     version: other.version.clone(),
//                     config: other.config.clone(),
//                 }),
//             )
//         }
//     }
// }
