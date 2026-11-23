//! Comparing our side with a reference list, and the report (summary and markdown).

use super::*;

/// Where a reference item is on our side; `Same` and `Elsewhere` carry the index of our item.
pub(super) enum Placement {
    Same(usize),
    Elsewhere(usize),
    Missing,
}

pub(super) struct Report {
    pub(super) area: Area,
    /// Our items, which `Placement` points into.
    pub(super) ours: Vec<OurItem>,
    /// (reference path, where it is in CFA4).
    pub(super) items: Vec<(Vec<String>, Placement)>,
    pub(super) out_of_scope: BTreeMap<&'static str, usize>,
    pub(super) ours_only: Vec<Vec<String>>,
}

/// [`compare_scoped`] without an extra out-of-scope list (the tests).
#[cfg(test)]
pub(super) fn compare(area: Area, reference: &[Vec<String>], ours: &[OurItem]) -> Report {
    compare_scoped(area, reference, ours, &[])
}

/// [`compare`], also counting out of scope every reference item under one of the `extra` path
/// prefixes (the schema-1 `outOfScope` list).
pub(super) fn compare_scoped(area: Area, reference: &[Vec<String>], ours: &[OurItem], extra: &[Vec<String>]) -> Report {
    let prefixes: Vec<Vec<String>> = extra.iter().map(|p| p.iter().map(|s| norm(s)).collect()).collect();
    let key = |p: &[String]| p.iter().map(|s| norm(s)).collect::<Vec<_>>();
    let mut by_path: HashMap<Vec<String>, usize> = HashMap::new();
    let mut by_label: HashMap<String, usize> = HashMap::new();
    for (i, it) in ours.iter().enumerate() {
        by_path.entry(key(&it.path)).or_insert(i);
        if let Some(l) = it.path.last() {
            by_label.entry(norm(l)).or_insert(i);
        }
    }
    let mut report = Report { area, ours: ours.to_vec(), items: Vec::new(), out_of_scope: BTreeMap::new(), ours_only: Vec::new() };
    let mut seen = HashSet::new();
    for p in reference {
        if !seen.insert(key(p)) {
            continue;
        }
        let normalised: Vec<String> = p.iter().map(|s| norm(s)).collect();
        let why = out_of_scope(area, p).or_else(|| prefixes.iter().any(|x| normalised.starts_with(x)).then_some("listed out of scope by the reference"));
        if let Some(why) = why {
            *report.out_of_scope.entry(why).or_default() += 1;
            continue;
        }
        let label = p.last().map(|l| norm(l)).unwrap_or_default();
        // a bare name (no folder) has no place to compare: found is all it can be
        let placement = match (by_path.get(&key(p)), by_label.get(&label)) {
            (Some(&i), _) => Placement::Same(i),
            (None, Some(&i)) if p.len() == 1 => Placement::Same(i),
            (None, Some(&i)) => Placement::Elsewhere(i),
            (None, None) => Placement::Missing,
        };
        report.items.push((p.clone(), placement));
    }
    let ref_labels: HashSet<String> = reference.iter().filter_map(|p| p.last().map(|l| norm(l))).collect();
    report.ours_only = ours.iter().filter(|it| it.path.last().is_some_and(|l| !ref_labels.contains(&norm(l)))).map(|it| it.path.clone()).collect();
    report
}

pub(super) fn pct(n: usize, of: usize) -> f64 {
    if of == 0 { 0.0 } else { n as f64 * 100.0 / of as f64 }
}

pub(super) fn show(p: &[String]) -> String {
    p.join(" ▸ ")
}

impl Report {
    pub(super) fn counts(&self) -> (usize, usize, usize) {
        let same = self.items.iter().filter(|(_, pl)| matches!(pl, Placement::Same(_))).count();
        let elsewhere = self.items.iter().filter(|(_, pl)| matches!(pl, Placement::Elsewhere(_))).count();
        (same, elsewhere, self.items.len().saturating_sub(same + elsewhere))
    }

    /// Reference items whose label we have anywhere, in percent of the items in scope.
    pub(super) fn presence(&self) -> f64 {
        let (same, elsewhere, _) = self.counts();
        pct(same + elsewhere, self.items.len())
    }

    /// Our item a reference item was found as, if it was.
    pub(super) fn found(&self, pl: &Placement) -> Option<&OurItem> {
        match pl {
            Placement::Same(i) | Placement::Elsewhere(i) => self.ours.get(*i),
            Placement::Missing => None,
        }
    }

    /// Reference items found with the given tier, with our item.
    pub(super) fn with_tier(&self, tier: Tier) -> Vec<(&[String], &OurItem)> {
        self.items.iter().filter_map(|(p, pl)| self.found(pl).filter(|it| it.tier == tier).map(|it| (p.as_slice(), it))).collect()
    }

    /// How many present items have each tier, in the order of [`Tier::ALL`].
    pub(super) fn tier_counts(&self) -> [usize; 4] {
        let mut c = [0; 4];
        for (_, pl) in &self.items {
            if let Some(it) = self.found(pl)
                && let Some(n) = c.get_mut(it.tier.index())
            {
                *n += 1;
            }
        }
        c
    }

    pub(super) fn summary(&self) -> String {
        let (same, elsewhere, missing) = self.counts();
        let n = self.items.len();
        let [exact, approximate, stub, unrated] = self.tier_counts();
        format!(
            "{}: presence {}/{n} ({:.1}%), same place {same}/{n} ({:.1}%), elsewhere {elsewhere}, missing {missing}, \
             fidelity exact {exact} / approximate {approximate} / stub {stub} / unrated {unrated}, out of scope {}",
            self.area.name(),
            same + elsewhere,
            self.presence(),
            pct(same, n),
            self.out_of_scope.values().sum::<usize>()
        )
    }

    pub(super) fn markdown(&self, reference: &str, ours: usize) -> String {
        let (same, elsewhere, missing) = self.counts();
        let n = self.items.len();
        let area = self.area;
        let (title, column) = match area {
            Area::Menus | Area::MenusEgui => ("Menu", "Menu"),
            Area::Commands => ("Command", "Menu"),
            Area::Panels => ("Panel", "Group"),
            Area::Effects => ("Effect", "Folder"),
            Area::Transitions => ("Transition", "Folder"),
        };
        let mut s = format!("# {title} parity checklist\n\n");
        s.push_str(&format!(
            "Generated by `cargo xtask parity --area {}` from `{reference}` ({n} items in scope, {} out of scope) against \
             CFA4's {} ({ours} items). *Same place*: we have the label at the same {place}; *elsewhere*: under \
             another {place}; *missing*: nowhere. A label match is presence only, not behaviour.{}\n\n",
            area.name(),
            self.out_of_scope.values().sum::<usize>(),
            area.ours(),
            match area {
                Area::Menus | Area::MenusEgui => "",
                Area::Panels => " A panel is matched by its title. A panel that is only a placeholder dock, or only listed in the Window menu, is an automatic stub: present, but not the real panel.",
                Area::Commands => " A bare name with no menu in the reference is matched by label alone; queries are left out.",
                Area::Effects | Area::Transitions => " A bare name with no folder in the reference is matched by name alone.",
            },
            place = area.place(),
        ));
        s.push_str(&format!("| {column} | Items | Same place | Elsewhere | Missing | Presence |\n|---|---|---|---|---|---|\n"));
        let mut per: Vec<(String, [usize; 3])> = Vec::new();
        for (p, pl) in &self.items {
            let top = area.group(p);
            let i = match per.iter().position(|(t, _)| *t == top) {
                Some(i) => i,
                None => {
                    per.push((top, [0; 3]));
                    per.len() - 1
                }
            };
            let col = match pl {
                Placement::Same(_) => 0,
                Placement::Elsewhere(_) => 1,
                Placement::Missing => 2,
            };
            if let Some(c) = per.get_mut(i).and_then(|(_, c)| c.get_mut(col)) {
                *c += 1;
            }
        }
        for (top, [a, b, c]) in &per {
            let total = a + b + c;
            s.push_str(&format!("| {top} | {total} | {a} | {b} | {c} | {:.1}% |\n", pct(a + b, total)));
        }
        s.push_str(&format!("| **Total** | **{n}** | **{same}** | **{elsewhere}** | **{missing}** | **{:.1}%** |\n", self.presence()));
        s.push_str(&format!("\n## Missing ({missing})\n\n"));
        for (p, _) in self.items.iter().filter(|(_, pl)| matches!(pl, Placement::Missing)) {
            s.push_str(&format!("- [ ] {}\n", show(p)));
        }
        s.push_str(&format!("\n## Elsewhere in CFA4 ({elsewhere})\n\n"));
        for (p, pl) in &self.items {
            if let Placement::Elsewhere(i) = pl
                && let Some(at) = self.ours.get(*i)
            {
                s.push_str(&format!("- {} → {}\n", show(p), show(&at.path)));
            }
        }
        s.push_str(&self.fidelity_markdown());
        s.push_str("\n## Out of scope\n\n");
        for (why, k) in &self.out_of_scope {
            s.push_str(&format!("- {why}: {k}\n"));
        }
        s.push_str(&format!("\n## CFA4 only ({})\n\n", self.ours_only.len()));
        for p in &self.ours_only {
            s.push_str(&format!("- {}\n", show(p)));
        }
        s
    }

    /// The fidelity section: counts per group and tier of the items we have, and the approximate and
    /// stub items with their notes. A tier is an annotation, not a behaviour test (gap G1).
    fn fidelity_markdown(&self) -> String {
        let mut s = String::from(
            "\n## Fidelity\n\nHow faithful the items we *have* are. A tier is a committed annotation (`xtask/parity/fidelity.json`) \
             or an automatic stub, **not a behaviour test** (docs/gaps.md G1). An item with no annotation is *unrated*, not exact: \
             `exact` needs evidence.\n\n",
        );
        s.push_str("| Group | Present | Exact | Approximate | Stub | Unrated |\n|---|---|---|---|---|---|\n");
        let mut per: BTreeMap<String, [usize; 4]> = BTreeMap::new();
        for (p, pl) in &self.items {
            if let Some(it) = self.found(pl)
                && let Some(n) = per.entry(self.area.group(p)).or_default().get_mut(it.tier.index())
            {
                *n += 1;
            }
        }
        for (group, c) in &per {
            let [e, a, st, u] = *c;
            s.push_str(&format!("| {group} | {} | {e} | {a} | {st} | {u} |\n", e + a + st + u));
        }
        let [e, a, st, u] = self.tier_counts();
        s.push_str(&format!("| **Total** | **{}** | **{e}** | **{a}** | **{st}** | **{u}** |\n", e + a + st + u));
        for tier in [Tier::Approximate, Tier::Stub] {
            let items = self.with_tier(tier);
            if items.is_empty() {
                continue;
            }
            let title = if tier == Tier::Stub { "Stub" } else { "Approximate" };
            s.push_str(&format!("\n### {title} ({})\n\n", items.len()));
            for (p, it) in items {
                s.push_str(&format!("- {} → {}: {}\n", show(p), show(&it.path), it.note));
            }
        }
        s
    }
}

/// The `--area all` report: a summary table of the areas, then every area's report one level down.
pub(super) fn all_markdown(root: &Path, outcomes: &[Outcome]) -> String {
    let mut s = String::from(
        "# Parity report\n\nGenerated by `cargo xtask parity --area all`. Presence is a label match, not behaviour; a fidelity tier \
         is an annotation, not a behaviour test (docs/gaps.md G1), and an item with no annotation is unrated, not exact.\n\n",
    );
    s.push_str(
        "| Area | Reference | In scope | Present | Exact | Approximate | Stub | Unrated | Missing | Presence |\n|---|---|---|---|---|---|---|---|---|---|\n",
    );
    for o in outcomes {
        match &o.measured {
            Some((r, source)) => {
                let (same, elsewhere, missing) = r.counts();
                let [e, a, st, u] = r.tier_counts();
                s.push_str(&format!(
                    "| {} | {} | {} | {} | {e} | {a} | {st} | {u} | {missing} | {:.1}% |\n",
                    o.area.name(),
                    source.label(root),
                    r.items.len(),
                    same + elsewhere,
                    r.presence()
                ));
            }
            None => s.push_str(&format!("| {} | not measured (no reference) | | | | | | | | |\n", o.area.name())),
        }
    }
    for o in outcomes {
        s.push('\n');
        match &o.measured {
            Some((r, source)) => {
                for line in r.markdown(&source.label(root), o.ours).lines() {
                    // one level down, so the areas nest under the report title
                    if line.starts_with('#') {
                        s.push('#');
                    }
                    s.push_str(line);
                    s.push('\n');
                }
            }
            None => s.push_str(&format!("## {} parity\n\nNo reference: not measured.\n", o.area.name())),
        }
    }
    s
}
