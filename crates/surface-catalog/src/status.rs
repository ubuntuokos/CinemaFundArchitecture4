//! Feature and model status from the catalogue and the reports (§3.2, §4.3) and the editor gate
//! (§3.4).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, Model};
use crate::model::{FeatureStatus, ModelStatus};
use crate::report::{Category, Outcome, Report};

/// The adapters this build contains: family id → major version.
pub type Adapters = BTreeMap<String, u64>;

/// The evaluated state of one feature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureState {
    pub status: FeatureStatus,
    /// The report the status comes from, when one does.
    #[serde(default)]
    pub report: Option<String>,
    /// Date of the newest report that tested the feature.
    #[serde(default)]
    pub last_tested: Option<String>,
    /// The only reports are for another adapter major version (§4.3: "re-verification needed").
    #[serde(default)]
    pub reverify: bool,
    /// Firmware the VERIFIED state was measured with (§4.3: shown when the device differs).
    #[serde(default)]
    pub firmware: Option<String>,
}

/// The evaluated state of one model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelState {
    pub id: String,
    pub status: ModelStatus,
    pub features: BTreeMap<String, FeatureState>,
    /// Why the model is BLOCKED, when it is.
    #[serde(default)]
    pub blocked: Option<String>,
}

fn can_verify(model: &Model, category: Category, feature: &str) -> bool {
    match category {
        Category::Cfa4Hil => true,
        Category::Cfa4Ci => model.protocol_features.iter().any(|f| f == feature),
        Category::External => false,
    }
}

/// Evaluates one model.
///
/// `reports` may hold reports of any model; only those of this one are used. The newest report
/// (by date, then by order) that tested a feature decides: a pass from a report that can verify
/// it makes it VERIFIED (PARTIAL when the adapter declares PARTIAL), anything else leaves the
/// declaration. A report for another adapter major version never verifies (§4.3).
pub fn evaluate(model: &Model, reports: &[(String, Report)], adapters: &Adapters) -> ModelState {
    let declared = model.declared_features();
    let current_major = model.adapter.as_ref().and_then(|a| adapters.get(a)).copied();
    let mut features = BTreeMap::new();
    for (id, &declared_status) in &declared {
        let mut state = FeatureState { status: declared_status, report: None, last_tested: None, reverify: false, firmware: None };
        let fixed = matches!(declared_status, FeatureStatus::NotApplicable | FeatureStatus::Unsupported | FeatureStatus::Blocked);
        let mut newest: Option<(&str, &str, &Report, Outcome)> = None;
        let mut stale = false;
        for (origin, r) in reports.iter().filter(|(_, r)| r.device.model == model.id) {
            let Some(res) = r.results.iter().find(|x| &x.feature == id) else { continue };
            if !can_verify(model, r.category, id) {
                continue;
            }
            if current_major.is_none() || r.environment.adapter.major() != current_major {
                stale = true;
                continue;
            }
            if newest.is_none_or(|(_, d, _, _)| r.date.as_str() >= d) {
                newest = Some((origin.as_str(), r.date.as_str(), r, res.result));
            }
        }
        if let Some((origin, date, r, outcome)) = newest {
            state.last_tested = Some(date.to_owned());
            if outcome == Outcome::Pass && !fixed {
                state.status = if declared_status == FeatureStatus::Partial { FeatureStatus::Partial } else { FeatureStatus::Verified };
                state.report = Some(origin.to_owned());
                state.firmware = r.device.firmware.clone();
            }
        } else if stale && !fixed {
            state.reverify = true;
        }
        features.insert(id.clone(), state);
    }
    let blocked = model.blocked().map(|b| b.reason.clone());
    let status = model_status(model, current_major.is_some(), blocked.is_some(), &features);
    ModelState { id: model.id.clone(), status, features, blocked }
}

/// §3.2, in order; NOT_APPLICABLE features are left out.
fn model_status(model: &Model, has_adapter: bool, blocked: bool, features: &BTreeMap<String, FeatureState>) -> ModelStatus {
    if !has_adapter {
        return if model.unavailable { ModelStatus::Unavailable } else { ModelStatus::Planned };
    }
    if blocked {
        return ModelStatus::Blocked;
    }
    let relevant = || features.values().filter(|f| f.status != FeatureStatus::NotApplicable);
    let baseline_verified = !model.baseline.is_empty() && model.baseline.iter().all(|b| features.get(b).is_some_and(|f| f.status == FeatureStatus::Verified));
    if baseline_verified && !relevant().any(|f| f.status == FeatureStatus::Untested) {
        return ModelStatus::Verified;
    }
    if relevant().any(|f| f.status == FeatureStatus::Verified) {
        return ModelStatus::Partial;
    }
    ModelStatus::Experimental
}

/// Evaluates the whole catalogue, in catalogue order.
pub fn evaluate_all(catalog: &Catalog, reports: &[(String, Report)], adapters: &Adapters) -> Vec<ModelState> {
    catalog.models.iter().map(|m| evaluate(m, reports, adapters)).collect()
}

/// What the mapping editor may do with a feature (§3.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    /// Assignable, no mark.
    Assignable,
    /// Assignable, with a warning icon and the limitation text.
    AssignableWithWarning,
    /// Assignable only while "Experimental features" is on; marked "experimental".
    Experimental,
    /// Shown greyed out with the reason; not assignable.
    Disabled,
    /// Not shown at all.
    Hidden,
}

/// The gate for a feature status, given the "Experimental features" switch.
pub fn gate(status: FeatureStatus, experimental: bool) -> Gate {
    match status {
        FeatureStatus::Verified => Gate::Assignable,
        FeatureStatus::Partial => Gate::AssignableWithWarning,
        FeatureStatus::Untested if experimental => Gate::Experimental,
        FeatureStatus::Untested | FeatureStatus::Blocked | FeatureStatus::Unsupported => Gate::Disabled,
        FeatureStatus::NotApplicable => Gate::Hidden,
    }
}
