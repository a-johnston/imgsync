use std::collections::HashMap;
use std::fs::{DirEntry, read_dir};
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Local};
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use regex::Regex;
use serde::Deserialize;

use crate::plan;
use crate::util;

pub fn get_files_for_dirs<P>(
    dirs: impl ParallelIterator<Item = P>,
) -> impl ParallelIterator<Item = DirEntry>
where
    P: AsRef<Path>,
{
    dirs.flat_map_iter(move |dir| read_dir(dir).into_iter().flatten())
        .filter_map(|entry| entry.ok())
}

#[derive(Deserialize)]
pub struct FilePattern {
    dir_pattern: String,
    #[serde(default)]
    file_regex: String,
}

impl FilePattern {
    pub fn expand_paths(&mut self) {
        self.dir_pattern = util::expand_homedir(&self.dir_pattern);
    }

    pub fn match_files_by_dir(&self) -> impl ParallelIterator<Item = Vec<FileGroup>> {
        let regex = if self.file_regex.is_empty() {
            None
        } else {
            Regex::new(&self.file_regex).ok()
        };
        // Each glob-matched directory is treated as an independent source so that
        // per-source cutoffs work correctly when a pattern matches multiple SD cards.
        glob::glob(&self.dir_pattern)
            .into_par_iter()
            .flat_map_iter(|paths| paths.flatten())
            // https://github.com/rust-lang/regex/blob/0d0023e41/PERFORMANCE.md#using-a-regex-from-multiple-threads
            .map(move |dir| {
                get_groups(
                    get_files_for_dirs(rayon::iter::once(dir))
                        .map_with(regex.clone(), |r, entry| FileInfo::get_from_dir_entry(entry, r))
                        .filter_map(|f| f),
                )
            })
    }
}

#[derive(Clone)]
pub struct FileFormatArgs {
    name: String,
    parent: String,
    extensions: String,
}

impl FileFormatArgs {
    fn new(p: &PathBuf, file_matcher: &Option<Regex>) -> Option<Self> {
        let file_name = util::path_fname(p);
        let mut parent = p.parent().map(util::path_fname).unwrap_or("");
        let (mut name, mut extensions) = file_name
            .split_once('.')
            .map(|(a, b)| (a, b.trim_start_matches(".")))
            .unwrap_or((file_name, ""));
        if let Some(regex) = file_matcher {
            if let Some(captures) = regex.captures(file_name) {
                (captures.name("name")).inspect(|s| name = s.as_str());
                (captures.name("parent")).inspect(|s| parent = s.as_str());
                (captures.name("extensions")).inspect(|s| extensions = s.as_str());
            } else {
                return None;
            }
        }
        Some(Self {
            name: name.to_string(),
            parent: parent.to_string(),
            extensions: extensions.trim_start_matches(".").to_string(),
        })
    }

    fn format(&self, s: &str, suffix: &str) -> String {
        s.replace("{name}", &self.name)
            .replace("{parent}", &self.parent)
            .replace("{suffix}", suffix)
            .replace("{extensions}", &self.extensions)
    }
}

#[derive(Clone)]
pub struct FileInfo {
    pub path: PathBuf,
    pub modified: SystemTime,
    pub size: u64,
    pub format_args: FileFormatArgs,
}

impl FileInfo {
    pub fn get_from_dir_entry(dir_entry: DirEntry, file_matcher: &Option<Regex>) -> Option<Self> {
        if let Ok(metadata) = dir_entry.metadata()
            && metadata.is_file()
            && let Some(format_args) = FileFormatArgs::new(&dir_entry.path(), file_matcher)
        {
            Some(Self {
                path: dir_entry.path(),
                modified: metadata.modified().unwrap(),
                size: metadata.len(),
                format_args,
            })
        } else {
            None
        }
    }

    fn content_matches(&self, other: &Self) -> bool {
        // This is a bad heuristic although it works for me. Replace/combine with cheap file hash?
        self.size == other.size
            && self.format_args.extensions == other.format_args.extensions
            && util::systime_delta(self.modified, other.modified).as_secs_f64() < 0.00001
    }
}

impl PartialEq for FileInfo {
    fn eq(&self, other: &Self) -> bool {
        self.modified == other.modified
            && self.format_args.name == other.format_args.name
            && self.format_args.extensions == other.format_args.extensions
    }
}

impl Hash for FileInfo {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.modified.hash(state);
        self.format_args.name.hash(state);
        self.format_args.extensions.hash(state);
    }
}

impl Eq for FileInfo {}

pub struct FileGroup {
    pub files: Vec<FileInfo>,
}

impl FileGroup {
    pub fn empty() -> Self {
        Self { files: vec![] }
    }

    fn get_ts(&self) -> DateTime<Local> {
        // Uses the earliest modified time in the group. For groups containing
        // related files (e.g. image + sidecar), the image file is typically
        // modified earlier and its time best reflects the capture time.
        self.files.iter().map(|f| f.modified).min().unwrap().into()
    }

    pub fn get_moves(
        &self,
        destination: &Destination,
        suffix: u64,
    ) -> impl Iterator<Item = (PathBuf, &FileInfo)> {
        let ts = self.get_ts();
        let dir: PathBuf = ts.format(&destination.dir_pattern).to_string().into();
        let format = ts.format(&destination.file_pattern).to_string();
        self.files.iter().map(move |file| {
            let suffix = if suffix == 0 {
                String::new()
            } else {
                format!("_{}", suffix)
            };
            let file_name = file.format_args.format(&format, &suffix);
            (util::path_with_push(&dir, file_name.as_str()), file)
        })
    }

    pub fn get_unique_suffix(
        &self,
        existing: &HashMap<PathBuf, FileInfo>,
        destination: &Destination,
    ) -> u64 {
        let mut suffix = 0;
        while self.get_moves(destination, suffix).any(|(k, v)| {
            (existing.get(&k))
                .map(|other| !v.content_matches(other))
                .unwrap_or(false)
        }) {
            if !destination.file_pattern.contains("{suffix}") {
                panic!("Cannot ensure uniqueness: no {suffix} in output file format")
            }
            suffix += 1;
        }
        suffix
    }
}

#[derive(Deserialize)]
pub struct Destination {
    dir_pattern: String,
    file_pattern: String,
}

impl Destination {
    pub fn expand_paths(&mut self) {
        self.dir_pattern = util::expand_homedir(&self.dir_pattern);
    }

    pub fn get_dirs<'a>(&'a self, groups: &'a [&'a FileGroup]) -> impl ParallelIterator<Item = PathBuf> + 'a {
        groups.par_iter().map(|g| g.get_ts().format(&self.dir_pattern).to_string().into())
    }

    fn find_source_cutoff(&self, groups: &[FileGroup], existing: &HashMap<PathBuf, FileInfo>) -> Option<SystemTime> {
        groups.iter()
            .filter(|g| {
                let suffix = g.get_unique_suffix(existing, self);
                g.get_moves(self, suffix).all(|(dest_path, _)| existing.contains_key(&dest_path))
            })
            .map(|g| g.files.iter().map(|f| f.modified).min().unwrap())
            .max()
    }

    pub fn plan_moves(
        &self,
        plan: &mut plan::Plan,
        source_groups: &[Vec<FileGroup>],
        existing: &mut HashMap<PathBuf, FileInfo>,
    ) {
        for groups in source_groups {
            let cutoff = self.find_source_cutoff(groups, existing);
            for group in groups {
                if cutoff.map_or(false, |c| {
                    group.files.iter().map(|f| f.modified).min().unwrap() <= c
                }) {
                    continue;
                }
                let suffix = group.get_unique_suffix(existing, self);
                for (dest, file) in group.get_moves(self, suffix) {
                    if !existing.contains_key(&dest) {
                        existing.insert(dest.clone(), file.clone());
                        plan.add_move(file, &dest);
                    }
                }
            }
        }
    }
}

pub fn get_groups(files: impl ParallelIterator<Item = FileInfo>) -> Vec<FileGroup> {
    files
        .fold(HashMap::new, |mut groups, file| {
            let key = file.path.file_prefix().unwrap().to_owned();
            let group = groups.entry(key).or_insert_with(FileGroup::empty);
            group.files.push(file);
            groups
        })
        .reduce(HashMap::new, |mut map_a, mut map_b| {
            util::map_update(&mut map_a, &mut map_b, |a, b| {
                a.files.extend_from_slice(&b.files)
            });
            map_a
        })
        .drain()
        .map(|(_, v)| v)
        .collect()
}
