//! KDMs (ST 430-1) for an encrypted DCP made by [`crate::Dcpomatic::make_dcp`], with
//! `dcpomatic2_kdm_cli create <film>`: the content keys come from the film directory's
//! `metadata.xml`, the recipient is a projector (media block) certificate file.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::request::{absolute, default_config_dir, text};
use crate::{Control, Dcpomatic, DcpomaticError, KDM_CLI, Phase, create_private_dir, process};

/// The KDM formulation (`--formulation`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Formulation {
    /// Modified Transitional 1 (the common one).
    #[default]
    ModifiedTransitional1,
    /// Modified Transitional 1 with several trusted devices.
    MultipleModifiedTransitional1,
    /// DCI, any trusted device.
    DciAny,
    /// DCI, only the listed trusted devices.
    DciSpecific,
}

impl Formulation {
    fn arg(self) -> &'static str {
        match self {
            Formulation::ModifiedTransitional1 => "modified-transitional-1",
            Formulation::MultipleModifiedTransitional1 => "multiple-modified-transitional-1",
            Formulation::DciAny => "dci-any",
            Formulation::DciSpecific => "dci-specific",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValidityUnit {
    Hours,
    Days,
    Weeks,
    Years,
}

/// How long the KDM is valid after `valid_from`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Validity {
    /// `count` hours, days, weeks or years.
    For { count: u32, unit: ValidityUnit },
    /// Until this UTC time, `YYYY-MM-DD HH:MM:SS`.
    Until(String),
}

/// A KDM to make.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KdmRequest {
    /// The film directory of an encrypted DCP made with DCP-o-matic.
    pub film_dir: PathBuf,
    /// The recipient: a PEM file with the projector's (media block's) certificate or chain; the
    /// leaf certificate is used.
    pub projector_certificate: PathBuf,
    /// The folder the KDM files go into (created when missing).
    pub output: PathBuf,
    /// Cinema and screen names, used in the KDM file names.
    #[serde(default)]
    pub cinema: Option<String>,
    #[serde(default)]
    pub screen: Option<String>,
    /// Start of validity, UTC `YYYY-MM-DD HH:MM:SS`; unset: now.
    #[serde(default)]
    pub valid_from: Option<String>,
    pub validity: Validity,
    #[serde(default)]
    pub formulation: Formulation,
    /// Further trusted devices' certificate files (for the multiple / DCI-specific formulations).
    #[serde(default)]
    pub trusted_devices: Vec<PathBuf>,
    #[serde(default)]
    pub disable_forensic_marking_picture: bool,
    #[serde(default)]
    pub disable_forensic_marking_audio: bool,
    /// DCP-o-matic's configuration directory (the KDM signing chain), as in
    /// [`crate::DcpRequest::config_dir`].
    #[serde(default)]
    pub config_dir: Option<PathBuf>,
}

/// What a KDM run wrote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KdmOutcome {
    /// The KDM files, sorted.
    pub files: Vec<PathBuf>,
}

/// `YYYY-MM-DD HH:MM:SS`, digits in the right places (DCP-o-matic parses the value itself).
fn is_time(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 19
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b' ',
            13 | 16 => *c == b':',
            _ => c.is_ascii_digit(),
        })
}

fn name_ok(s: &str) -> bool {
    !s.trim().is_empty() && !s.chars().any(char::is_control) && !s.starts_with('-')
}

impl KdmRequest {
    pub(crate) fn args(&self) -> Result<(Vec<String>, PathBuf, Option<PathBuf>), DcpomaticError> {
        let bad = |m: String| Err(DcpomaticError::Invalid(m));
        let film = absolute(&self.film_dir)?;
        let metadata = std::fs::read_to_string(film.join("metadata.xml"))
            .map_err(|_| DcpomaticError::Invalid(format!("{} is not a DCP-o-matic film directory (no metadata.xml)", film.display())))?;
        if !metadata.contains("<Encrypted>1</Encrypted>") {
            return bad(format!("the DCP in {} is not encrypted; KDMs are only for encrypted DCPs", film.display()));
        }
        let cert = absolute(&self.projector_certificate)?;
        if !cert.is_file() {
            return bad(format!("projector certificate {} does not exist", cert.display()));
        }
        let output = absolute(&self.output)?;
        if output.exists() && !output.is_dir() {
            return bad(format!("{} is not a folder", output.display()));
        }
        let mut a: Vec<String> = vec!["--output".into(), text(&output)?, "--verbose".into()];
        a.push("--valid-from".into());
        match &self.valid_from {
            None => a.push("now".into()),
            Some(t) if is_time(t) => a.push(t.clone()),
            Some(t) => return bad(format!("valid from {t:?} is not YYYY-MM-DD HH:MM:SS")),
        }
        match &self.validity {
            Validity::For { count: 0, .. } => return bad("the validity must be at least 1".into()),
            Validity::For { count, unit } => {
                a.push("--valid-duration".into());
                let unit = match unit {
                    ValidityUnit::Hours => "hours",
                    ValidityUnit::Days => "days",
                    ValidityUnit::Weeks => "weeks",
                    ValidityUnit::Years => "years",
                };
                a.push(format!("{count} {unit}"));
            }
            Validity::Until(t) if is_time(t) => {
                if self.valid_from.as_ref().is_some_and(|f| f >= t) {
                    return bad(format!("valid until {t} is not after valid from"));
                }
                a.push("--valid-to".into());
                a.push(t.clone());
            }
            Validity::Until(t) => return bad(format!("valid until {t:?} is not YYYY-MM-DD HH:MM:SS")),
        }
        a.push("--formulation".into());
        a.push(self.formulation.arg().into());
        a.push("--projector-certificate".into());
        a.push(text(&cert)?);
        for (flag, v) in [("--cinema", &self.cinema), ("--screen", &self.screen)] {
            if let Some(v) = v {
                if !name_ok(v) {
                    return bad(format!("{flag} {v:?} is empty, starts with '-' or has a control character"));
                }
                a.push(flag.into());
                a.push(v.trim().into());
            }
        }
        for d in &self.trusted_devices {
            let d = absolute(d)?;
            if !d.is_file() {
                return bad(format!("trusted device certificate {} does not exist", d.display()));
            }
            a.push("--trusted-device-certificate".into());
            a.push(text(&d)?);
        }
        if self.disable_forensic_marking_picture {
            a.push("--disable-forensic-marking-picture".into());
        }
        if self.disable_forensic_marking_audio {
            // `=0` (all channels): the bare option would take the next argument as its value
            a.push("--disable-forensic-marking-audio=0".into());
        }
        a.push("create".into());
        a.push(text(&film)?);
        let config = match &self.config_dir {
            Some(d) => Some(absolute(d)?),
            None => default_config_dir(),
        };
        Ok((a, output, config))
    }
}

fn xml_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xml")))
        .collect();
    v.sort();
    v
}

impl Dcpomatic {
    /// Makes the KDM(s) for an encrypted DCP made by [`Dcpomatic::make_dcp`]. Returns the KDM
    /// files written into the output folder by this run.
    pub fn make_kdm(&self, request: &KdmRequest) -> Result<KdmOutcome, DcpomaticError> {
        let Some(program) = &self.kdm else {
            return Err(DcpomaticError::Unavailable(format!("{KDM_CLI} was not found next to {}", self.cli.display())));
        };
        let (args, output, config) = request.args()?;
        self.version()?;
        std::fs::create_dir_all(&output)?;
        if let Some(cfg) = &config {
            create_private_dir(cfg)?;
        }
        // DCP-o-matic overwrites a KDM of the same name, so "new" means written during this run
        let started = std::time::SystemTime::now() - Duration::from_secs(2);
        let timeout = Duration::from_secs(10 * 60);
        process::run_tool(program, &args, config.as_deref(), Instant::now() + timeout, timeout, &Control::new(), Phase::Kdm)?;
        let files: Vec<PathBuf> =
            xml_files(&output).into_iter().filter(|f| std::fs::metadata(f).and_then(|m| m.modified()).is_ok_and(|t| t >= started)).collect();
        if files.is_empty() {
            return Err(DcpomaticError::Engine(format!("{KDM_CLI} finished but wrote no KDM into {}", output.display())));
        }
        Ok(KdmOutcome { files })
    }
}
