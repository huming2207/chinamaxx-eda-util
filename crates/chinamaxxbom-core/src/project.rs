use crate::board::{self, Board};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Edit {
    pub lcsc: Option<String>,
    pub dnp: Option<bool>,
    /// Additive assembly coordinates (world axes), mm / degrees.
    pub dx: f64,
    pub dy: f64,
    pub rotation: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub version: u32,
    pub board: PathBuf,
    pub source_sha256: String,
    pub edits: BTreeMap<String, Edit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement_board: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bom: Option<Board>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub source_hashes: BTreeMap<PathBuf, String>,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
impl Project {
    pub fn new(path: &Path) -> Result<Self> {
        let loaded = crate::input::load(path)?;
        let board = loaded.board.clone().unwrap_or_else(|| loaded.path.clone());
        let source_hashes = loaded
            .files
            .iter()
            .map(|p| Ok((p.clone(), hash(&std::fs::read(p)?))))
            .collect::<Result<_>>()?;
        Ok(Self {
            version: 2,
            source_sha256: hash(&std::fs::read(&board)?),
            board,
            edits: BTreeMap::new(),
            input: Some(loaded.path),
            placement_board: loaded.board,
            bom: Some(loaded.bom),
            source_hashes,
        })
    }
    /// Opening the same CAD input resumes its atomic JSON sidecar automatically.
    pub fn open(path: &Path) -> Result<(Self, PathBuf)> {
        if path.extension().is_some_and(|e| e == "json") {
            return Ok((Self::load(path)?, path.to_owned()));
        }
        let input = crate::input::resolve(path)?;
        let sidecar = PathBuf::from(format!("{}.chinamaxxbom.json", input.display()));
        if sidecar.exists() {
            return Ok((Self::load(&sidecar)?, sidecar));
        }
        let project = Self::new(&input)?;
        write_new(&sidecar, &serde_json::to_vec_pretty(&project)?)?;
        Ok((project, sidecar))
    }
    pub fn has_placement(&self) -> bool {
        self.version == 1 || self.placement_board.is_some()
    }
    /// Commit a supplier selection before reporting success to the UI.
    pub fn pick(&mut self, path: &Path, reference: &str, lcsc: &str) -> Result<()> {
        let mut next = self.clone();
        let mut edit = next.edits.get(reference).cloned().unwrap_or_default();
        edit.lcsc = Some(crate::lcsc_id(lcsc)?);
        next.edit(reference, edit)?;
        next.save(path)?;
        *self = next;
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        let mut p: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        ensure!(
            p.version == 1 || p.version == 2,
            "Unsupported project version {}",
            p.version
        );
        if p.board.is_relative() {
            p.board = path.parent().unwrap_or(Path::new(".")).join(&p.board);
        }
        let parent = path.parent().unwrap_or(Path::new("."));
        for value in [&mut p.input, &mut p.placement_board].into_iter().flatten() {
            if value.is_relative() {
                *value = parent.join(&*value);
            }
        }
        p.source_hashes = p
            .source_hashes
            .into_iter()
            .map(|(p, h)| (if p.is_relative() { parent.join(p) } else { p }, h))
            .collect();
        ensure!(
            p.version == 1 || p.bom.is_some(),
            "Version 2 requires a BOM snapshot"
        );
        p.snapshot()?;
        Ok(p)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.snapshot()?;
        atomic_write(path, &serde_json::to_vec_pretty(self)?)
    }
    pub fn source(&self) -> Result<String> {
        let bytes = std::fs::read(&self.board)?;
        ensure!(hash(&bytes)==self.source_sha256,"Board changed since this project was created. Create a new project and review assignments before exporting.");
        Ok(String::from_utf8(bytes)?)
    }
    pub fn snapshot(&self) -> Result<Board> {
        let source = self.source()?;
        for (path, expected) in &self.source_hashes {
            ensure!(hash(&std::fs::read(path).with_context(|| format!("Read source {}", path.display()))?) == *expected,
                "CAD source changed: {}. Saved picks are preserved; review them against a new import before exporting.", path.display());
        }
        let mut b = match &self.bom {
            Some(b) => b.clone(),
            None => board::parse(&source)?,
        };
        for (reference, e) in &self.edits {
            let p = b
                .parts
                .iter_mut()
                .find(|p| &p.reference == reference)
                .with_context(|| format!("Unknown reference {reference}"))?;
            ensure!(
                e.dx.is_finite() && e.dy.is_finite() && e.rotation.is_finite(),
                "Non-finite correction for {reference}"
            );
            if let Some(id) = &e.lcsc {
                p.lcsc = crate::lcsc_id(id)?;
            }
            if let Some(dnp) = e.dnp {
                p.dnp = dnp;
            }
            p.x += e.dx;
            p.y += e.dy;
            p.rotation = (p.rotation + e.rotation).rem_euclid(360.);
            ensure!(
                p.x.is_finite() && p.y.is_finite(),
                "Correction overflow for {reference}"
            );
        }
        Ok(b)
    }
    pub fn edit(&mut self, reference: &str, edit: Edit) -> Result<()> {
        let old = self.edits.insert(reference.into(), edit);
        if let Err(e) = self.snapshot() {
            if let Some(old) = old {
                self.edits.insert(reference.into(), old);
            } else {
                self.edits.remove(reference);
            }
            return Err(e);
        }
        Ok(())
    }
    pub fn write_board_copy(&self, path: &Path) -> Result<()> {
        self.snapshot()?;
        ensure!(
            self.has_placement(),
            "Board write-back requires a matching PCB"
        );
        let assignments = self
            .edits
            .iter()
            .filter_map(|(r, e)| e.lcsc.as_ref().map(|id| (r.clone(), id.clone())))
            .collect();
        let text = board::assigned_copy(&self.source()?, &assignments)?;
        write_new(path, text.as_bytes())
    }
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut f = tempfile::NamedTempFile::new_in(parent)?;
    f.write_all(bytes)?;
    f.as_file().sync_all()?;
    f.persist(path)?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut f = tempfile::NamedTempFile::new_in(parent)?;
    f.write_all(bytes)?;
    f.as_file().sync_all()?;
    f.persist_noclobber(path)
        .with_context(|| format!("Refusing to overwrite {}", path.display()))?;
    Ok(())
}
