use std::collections::HashMap;
use std::fs::copy;
use std::ops::Add;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use indicatif::{ProgressBar, ProgressStyle};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use crate::files::FileInfo;
use crate::util;

pub struct Plan {
    use_first_moves: bool,
    moves: HashMap<PathBuf, FileInfo>,
    first_moves: HashMap<FileInfo, PathBuf>,
}

fn copy_file(
    source: &FileInfo,
    dest: &PathBuf,
    bytes: &AtomicU64,
    errors: &AtomicU64,
    progress_bar: &ProgressBar,
) {
    progress_bar.inc(1);
    let new_bytes = copy(&source.path, dest).unwrap_or_else(|e| {
        println!("Error copying {:?} to {dest:?}: {e:?}", &source.path);
        errors.fetch_add(1, Ordering::Relaxed);
        0
    });
    let sum = (bytes.fetch_add(new_bytes, Ordering::Relaxed) + new_bytes) as f64;
    let rate = util::format_bytes(sum / progress_bar.elapsed().as_secs_f64());
    progress_bar.set_message(format!("{rate}/s {:>10}", util::format_bytes(sum)));
}

fn copy_files<'a>(
    prefix: &str,
    count: usize,
    it: impl ParallelIterator<Item = (&'a FileInfo, &'a PathBuf)>,
) {
    let msg = (prefix.to_string()).add("[{elapsed_precise}] {bar:40} {pos:>7}/{len:7} {msg}");
    let style = (ProgressStyle::with_template(&msg).unwrap()).progress_chars("##-");
    let progress_bar = ProgressBar::new(count as u64).with_style(style);
    let bytes = AtomicU64::new(0);
    let errors = AtomicU64::new(0);
    it.for_each(|(source, dest)| copy_file(source, dest, &bytes, &errors, &progress_bar));
    let time = progress_bar.elapsed();
    progress_bar.finish_and_clear();
    let errors = errors.load(Ordering::Relaxed);
    let bytes = bytes.load(Ordering::Relaxed);
    let rate = bytes as f64 / time.as_secs_f64();
    println!(
        "{}Copied: {}\tErrors: {}\tTime: {:.1?}\tRate: {}/s\tTotal: {}",
        prefix,
        count as u64 - errors,
        errors,
        time,
        util::format_bytes(rate),
        util::format_bytes(bytes as f64),
    );
}

impl Plan {
    pub fn new(use_first_dest_copies: bool) -> Self {
        Self {
            use_first_moves: use_first_dest_copies,
            moves: HashMap::new(),
            first_moves: HashMap::new(),
        }
    }

    pub fn add_move(&mut self, source: &FileInfo, dest: &Path) {
        if self.use_first_moves {
            if let Some(first_dest) = self.first_moves.get(source) {
                let new_file = FileInfo {
                    path: first_dest.clone(),
                    modified: source.modified,
                    size: source.size,
                    format_args: source.format_args.clone(),
                };
                self.moves.insert(dest.to_path_buf(), new_file);
            } else {
                self.first_moves.insert(source.clone(), dest.to_path_buf());
            }
        } else {
            self.moves.insert(dest.to_path_buf(), source.clone());
        }
    }

    pub fn get_first_moves(&self) -> impl ParallelIterator<Item = (&FileInfo, &PathBuf)> {
        self.first_moves.par_iter().map(|(k, v)| (k, v))
    }

    pub fn get_secondary_moves(&self) -> impl ParallelIterator<Item = (&FileInfo, &PathBuf)> {
        self.moves.par_iter().map(|(k, v)| (v, k))
    }

    pub fn log_moves(&self) {
        println!("Moves:");
        let mut sorted_moves: Vec<_> = self
            .get_first_moves()
            .chain(self.get_secondary_moves())
            .map(|(k, v)| (k.path.as_os_str(), v.as_os_str()))
            .collect();
        sorted_moves.sort();
        let max_source_len = &(sorted_moves.iter())
            .map(|(source, _)| source.len())
            .max()
            .unwrap();
        for (source, dest) in sorted_moves {
            println!("- {1:0$?} -> {2:?}", max_source_len + 2, source, dest);
        }
    }

    pub fn len(&self) -> usize {
        self.moves.len() + self.first_moves.len()
    }

    pub fn perform_moves(self) {
        if self.use_first_moves {
            if self.moves.is_empty() {
                copy_files("", self.first_moves.len(), self.get_first_moves());
            } else {
                let start = SystemTime::now();
                copy_files("[1/2] ", self.first_moves.len(), self.get_first_moves());
                copy_files("[2/2] ", self.moves.len(), self.get_secondary_moves());
                println!("Total Time: {:.1?}", start.elapsed().unwrap());
            }
        } else {
            copy_files("", self.moves.len(), self.get_secondary_moves());
        }
    }
}
