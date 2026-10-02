# Spec: Comic directory format normalization

## Objective

Normalize comic directory names to one format:

```text
[저자명] 제목 (1-5권 미완외)
```

The numeric range comes from the directory contents. Completion and extra-material
markers stay inside the final parentheses.

Target only `3_미정리`, `4_대기`, and `9_temp` under the configured root.

## Tech Stack

- Python 3.13
- Standard library only for metadata inference
- Existing bookstore clients for canonical titles

## Commands

```text
Dry run: python scripts/normalize_comic_directories.py
Polite dry run: python scripts/normalize_comic_directories.py --delay 1.2
Apply: python scripts/normalize_comic_directories.py --apply
Syntax: python -m py_compile scripts/normalize_comic_directories.py
Lint: ruff check scripts/normalize_comic_directories.py
```

## Project Structure

- `scripts/normalize_comic_directories.py`: scanning, bookstore lookup, reports, renames
- `docs/superpowers/specs/`: approved normalization rules
- `/tmp/comic-directory-normalization.tsv`: default dry-run report

## Code Style

Metadata is rendered in one place:

```python
suffix = f"(1-{last_number}{unit}{' ' + flags if flags else ''})"
```

- Use unpadded numbers.
- Use `화` when main episode files exist; otherwise use `권`.
- Keep an existing Korean directory author. Otherwise use Korean author spelling from
  contained files, another library directory, or a bookstore result, in that order.
- Do not put an English-only author name in a normalized directory name.
- Ignore Korean role labels such as `글` and `그림` when deciding whether an author
  name is Korean.

## Metadata Rules

1. Scan PDF and EPUB file names plus immediate child directory names.
2. Ignore files marked as extra material when calculating the main range.
3. Treat `외전`, `번외`, `특전`, `특별편`, `보너스`, `extra`, `special`, and
   `side story` as extra-material markers.
4. Append `외` only when main content and extra material both exist, or when the
   existing directory metadata already has an extra marker.
5. Preserve explicit `미완`, `연재`, missing-content, and stopped-series signals as
   `미완`.
6. Preserve explicit `완` or `완결` as `완`.
7. Leave the completion status empty when it cannot be established.
8. Use the lowest and highest detected main-content numbers as the range.
9. Skip a rename when a Korean author or range cannot be inferred safely.
10. Preserve intrinsic title numbers such as `K2` when removing a volume suffix.
11. Use the directory title when two or more numerically ordered files have no
    parseable title.
12. Treat trailing Latin aliases and parenthetical aliases as comparison-only title
    variants. Keep the bookstore title as the output title.

## Testing Strategy

- Run Python syntax validation and Ruff.
- Review the generated TSV before using `--apply`.
- Check representative volume, episode, mixed, incomplete, complete, and extra cases
  in the dry-run report.

## Boundaries

- Always: keep dry-run as the default and preserve an existing Korean author spelling.
- Always: wait at least `--delay` seconds after each completed live bookstore request.
- Always: cache raw per-store candidates. Cache empty results for seven days to avoid
  repeated bookstore traffic.
- Ask first: change the format, add dependencies, or infer completion from an online
  source.
- Never: rename directories without `--apply`, count image-page numbers as volumes or
  episodes, or replace an existing author with bookstore spelling.

## Success Criteria

- `[마츠모토 토모] 미녀는 야수 (01-05)` becomes
  `[마츠모토 토모] 미녀는 야수 (1-5권)`.
- `나루토 41-72권,외전 완결` becomes
  `[키시모토 마사시] 나루토 (41-72권 완외)` when a Korean author name can be
  resolved.
- An incomplete five-volume work with extra material ends with `(1-5권 미완외)`.
- A completed 120-episode work ends with `(1-120화 완)`.
- Unknown completion omits `완` and `미완`.
- Trailing bookstore periods are removed from canonical titles.
- Ambiguous ranges produce a skip status instead of a guessed directory name.
- The TSV reports actual PDF and EPUB counts and a specific skip reason.

## Open Questions

None. The user approved `[저자명] 제목 (1-5권 미완외)` on 2026-10-01.
