//! Everything the network page shows, computed from one snapshot: classification, connectivity and route
//! ownership. Built by the broker (`network.overview`) and rendered by the app.

use crate::classify::{Classification, classify};
use crate::connectivity::{ConnectivityLevel, global};
use crate::model::NetworkSnapshot;
use crate::routes::{DefaultRoute, VpnRouting, analyze};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkOverview {
    pub snapshot: NetworkSnapshot,
    /// Same order as `snapshot.interfaces`.
    pub classifications: Vec<Classification>,
    pub connectivity: ConnectivityLevel,
    pub ipv4_default: Option<DefaultRoute>,
    pub ipv6_default: Option<DefaultRoute>,
    pub vpns: Vec<VpnRouting>,
    pub ipv6_bypass: Option<u64>,
}

impl NetworkOverview {
    pub fn of(snapshot: NetworkSnapshot) -> Self {
        let classifications: Vec<Classification> =
            snapshot.interfaces.iter().map(|i| classify(i, snapshot.ras_connection(i))).collect();
        let routes = analyze(&snapshot, &classifications);
        Self {
            connectivity: global(&snapshot),
            classifications,
            ipv4_default: routes.ipv4_default,
            ipv6_default: routes.ipv6_default,
            vpns: routes.vpns,
            ipv6_bypass: routes.ipv6_bypass,
            snapshot,
        }
    }
}
