# imgsync

Basic utility for copying new files found in sources to structured destinations.
The specific use case this was designed for is quickly syncing new photos and videos
from an SD card into a folder-structured library as well as backup paths. This program
only reads and copies files - no files are removed or altered.

On first run, the binary creates a [config template](https://github.com/a-johnston/imgsync/blob/main/default_config.toml)
at `~/.config/imgsync/config.toml`. Sources and destinations are grouped into named
sections, which are processed independently. This lets different file types be synced
to different places. For example, a user on MacOS might have the following config:

```toml
[[sections]]
name = "photos"

[[sections.sources]]
dir_pattern = "/Volumes/*/DCIM/*/"
file_regex = "^\\w+[.](?i:jpg|jpeg|png|cr2|cr3|arw|nef|dng|xmp)$"

[[sections.destinations]]
dir_pattern = "~/Pictures/library/%Y/%b/"
file_pattern = "{parent}_{name}{suffix}.{extensions}"

[[sections]]
name = "videos"

[[sections.sources]]
dir_pattern = "/Volumes/*/PRIVATE/M4ROOT/CLIP/"
file_regex = "^\\w+[.](?i:mp4|mov|avi|mts)$"

[[sections.destinations]]
dir_pattern = "~/Pictures/video/%Y/%b/"
file_pattern = "{name}{suffix}.{extensions}"
```

# Multi-Destination Syncing

If multiple destinations are configured within a section, in some cases it can be helpful
to set `prefer_dest_copies = true`. This option uses the first destination for each file
as the source for copying to remaining destinations, under the assumption that the user's
primary storage has higher read speeds compared to an SD card.

Destinations are processed in-order so this option benefits from faster storage being
earlier in the config. It does not affect the final set of files or their contents.

# Migrating existing files

Named capture groups can be used within `[[sections.sources]]` to override what values
are used for formatting destination paths. Using the above config as an example, if one
wanted to add an additional folder level, a source can be added for the current destination
to copy those files to the new structure:

```toml
[[sections.sources]]
dir_pattern = "/Volumes/*/DCIM/*/"
file_regex = "^\\w+([.]\\w+)+$"

# The example path to migrate
[[sections.sources]]
dir_pattern = "~/Pictures/old_library/*/*/"
file_regex = "^(?<parent>\\w+)_(?<name>\\w+)([.]\\w+)+$"

[[sections.destinations]]
dir_pattern = "~/Pictures/new_library/%Y/%b/%d"
file_pattern = "{parent}_{name}{suffix}.{extensions}"
```

In this example, the named capture groups `parent` and `name` are matched by the regex
and used in place of the default values for that file (which are the name of the parent
directory and full file prefix respectively).

# File Matching

If two files are mapped to the same destination, or that destination already exists,
imgsync uses a basic heuristic to determine if those files should be considered the
same: the files must have the same size, the same extension, and modificatio times.
This works for the intended use case but may cause problems with others. In th future,
a content hash could be used for more robust matching.

If two files are determined to be matching, the copy is skipped. Otherwise, a unique
suffix value is incremented until the filename is unique.

# File Grouping

When scanning source files within a section, files with the same filename prefix are
put into a group together. Groups do not span multiple sources. When files are copied,
all files in a group share a common unique suffix and are placed in the same destination
folder as determined by the earliest modification time in the group. This ensures
metadata or sidecar files (e.g. XMP) are copied alongside the original source files.

# Todo

- Optimize copy planning for maximum throughput
  - Currently, `prefer_dest_copies` can improve throughput but because the copies are
    planned as two discrete steps, and because the first dest has greater bandwidth
    than the source, copies from the first dest can start being written to secondary
    dests as soon as they finish. Instead, this only starts after the entire first pass
    is complete. As a result, bandwidth use across the full plan is suboptimal for both
    the source and first dest. A good solution might be to invert the map of `<dest_path,
    source_file>` to a map of `<source_file, vec<dest_path>>`.
  - The current behavior of `prefer_dest_copies` does enable the source to be disconnected
    after the first pass (the first dest is determined by file, so a secondary dest missing
    files compared to the primary will be the "first dest" for those files and those copies
    will happen in the first pass). It's unclear if this is actually useful behavior but it
    would not be immediately compatible with the above bullet point's solution.
- Optional recovery of corrupted files in the case of failed/interrupted copies
- Optional automatic deleting of migrated files
- More robust file matching

# Why?

Because I wanted a simple command to run when I plug in my SD cards without needing to start
up darktable. Also it was getting a bit ridiculous browsing my existing library, which was
just a single folder. I also wanted an excuse to use rust again so I wouldn't forget everything
about it.
