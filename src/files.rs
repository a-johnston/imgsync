use std::collections::{HashMap, HashSet};
use std::fs::{DirEntry, read_dir};
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Local};
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use regex::Regex;
use serde::Deserialize;

use crate::util;
use crate::plan;

pub fn get_files_for_dirs<P>(
    dirs: impl ParallelIterator<Item = P>,
) -> impl ParallelIterator<Item = DirEntry>
where
    P: AsRef<Path>,
{
    dirs.flat_map_iter(move |dir| read_dir(dir).unwrap().into_iter())
        .flatten()
}

#[derive(Deserialize)]
pub struct FilePattern {
    dir_pattern: String,
    file_regex: String,
}

impl FilePattern {
    pub fn expand_paths(&mut self) {
        self.dir_pattern = util::expand_homedir(&self.dir_pattern);
    }

    pub fn match_files(&self) -> impl ParallelIterator<Item = FileInfo> {
        get_files_for_dirs(
            glob::glob(&self.dir_pattern)
                .into_par_iter()
                .flat_map_iter(|paths| paths.flatten()),
        )
        // https://github.com/rust-lang/regex/blob/0d0023e41/PERFORMANCE.md#using-a-regex-from-multiple-threads
        .map_with(Regex::new(&self.file_regex).ok(), |regex, dir_entry| {
            FileInfo::get_from_dir_entry(dir_entry, regex)
        })
        .filter_map(|f| f)
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
        let mut parent = (p.parent())
            .and_then(|p| Some(util::path_fname(p)))
            .unwrap_or("");
        let (mut name, mut extensions) = (file_name.split_once('.'))
            .and_then(|(a, b)| Some((a, b.trim_start_matches("."))))
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
            .replace("{suffix}", &suffix)
            .replace("{extensions}", &self.extensions)
    }
}

#[derive(Clone)]
pub struct FileInfo {
    pub path: PathBuf,
    pub created: SystemTime,
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
                // NB: If created metadata is not available, this program shouldn't run
                created: metadata.created().unwrap(),
                format_args: format_args,
            })
        } else {
            None
        }
    }

    fn matches(&self, other: &Self) -> bool {
        // This is a bad heuristic although it works for me. Replace/combine with cheap file hash?
        let delta = match self.created.duration_since(other.created) {
            Ok(d) => d,
            Err(e) => e.duration(),
        };
        return delta.as_secs_f32() < 0.001;
    }
}

impl PartialEq for FileInfo {
    fn eq(&self, other: &Self) -> bool {
        self.created == other.created && self.format_args.name == other.format_args.name
    }
}

impl Hash for FileInfo {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.created.hash(state);
        self.format_args.name.hash(state);
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
        // Uses the earliest available file creation time. Intuitively, editor
        // metadata is created after image files are, but needs to be moved to
        // the same destination for migrations.
        self.files.iter().map(|f| f.created).min().unwrap().into()
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
                .and_then(|other| Some(!v.matches(other)))
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
    ignore: Vec<FilePattern>,
}

impl Destination {
    pub fn expand_paths(&mut self) {
        self.dir_pattern = util::expand_homedir(&self.dir_pattern);
        (self.ignore.iter_mut()).for_each(|ignore| ignore.expand_paths());
    }

    pub fn get_dirs(&self, groups: &Vec<FileGroup>) -> impl ParallelIterator<Item = PathBuf> {
        (groups.par_iter()).map(|g| g.get_ts().format(&self.dir_pattern).to_string().into())
    }

    pub fn populate_moves(
        &self,
        groups: &mut Vec<FileGroup>,
        plan: &mut plan::Plan,
        existing: &mut HashMap<PathBuf, FileInfo>,
    ) {
        // Populates a map of moves for this destination. If maybe_first_moves is not None, it is populated
        // with the first instance of a file movement and that destination is used as a source for future
        // copies of that file. The contents of maybe_first_moves are processed before those of moves.
        let ignore: HashSet<_> = (self.ignore.par_iter())
            .flat_map(|ignore| ignore.match_files())
            .collect();
        for group in groups {
            let suffix = group.get_unique_suffix(existing, self);
            for (dest, file) in group.get_moves(self, suffix) {
                if !existing.contains_key(&dest) && !ignore.contains(file) {
                    existing.insert(dest.clone(), file.clone());
                    plan.add_move(file, &dest);
                }
            }
        }
    }
}

pub fn get_groups(files: impl ParallelIterator<Item = FileInfo>) -> Vec<FileGroup> {
    files
        .fold(
            || HashMap::new(),
            |mut groups, file| {
                let key = file.path.file_prefix().unwrap().to_owned();
                let group = groups.entry(key).or_insert_with(FileGroup::empty);
                group.files.push(file);
                groups
            },
        )
        .reduce(
            || HashMap::new(),
            |mut map_a, mut map_b| {
                util::map_update(&mut map_a, &mut map_b, |a, b| {
                    (*a).files.extend_from_slice(&b.files)
                });
                map_a
            },
        )
        .drain()
        .map(|(_, v)| v)
        .collect()
}
