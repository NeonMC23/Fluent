# Release Integrity: Historical Assets and Source Distribution

This document separates three different claims that are easy to confuse:

1. **The current source tree is clean.**
2. **The Git history still contains third-party assets.**
3. **No remote history has been rewritten or cleaned.**

Only the first is true in the working repository. The other two remain release
decisions.

## 1. Historical asset inventory

The affected paths are the two NLLB model directories removed from the working
tree in Cycle 7:

```text
models/nllb-200-1.3b/
  README.md, config.json, generation_config.json, gitattributes,
  pytorch_model.bin (1-byte placeholder), sentencepiece.bpe.model,
  special_tokens_map.json, tokenizer.json, tokenizer_config.json

models/nllb-200-3.3b/
  README.md, config.json, gitattributes,
  pytorch_model-00001-of-00003.bin, -00002, -00003 (1-byte placeholders),
  pytorch_model.bin.index.json, sentencepiece.bpe.model,
  special_tokens_map.json, tokenizer.json, tokenizer_config.json
```

There are 20 path entries per tree. Much of the content is duplicated between
the 1.3B and 3.3B directories, so Git stores **12 unique blobs totalling
~21.3 MiB**; a checkout of the affected tree expands to ~43 MB. The two largest
blobs are `tokenizer.json` (17,331,176 bytes) and
`sentencepiece.bpe.model` (4,852,054 bytes). No model weights were ever
committed; the `.bin` files are 1-byte placeholders.

### Introduction and removal

- Added 2026-10-23 in uploads `a170ea9`, `04fd9ac`, `59e7874`, followed by
  placeholder/index commits `1b0d3a6`, `ecce18b`, `5e615d0`, `50125d6`.
- Last tree containing them: `1e010e0` (Cycle 6 HEAD), the parent of the
  Cycle 7 removal commit `08b7418`.
- The current working tree contains only `models/README.md` (original MIT text).

### Reachable references

The asset-bearing commits are reachable from:

- local branches `main`, `finalization/cycle-2` through `cycle-7` (and the
  current `cycle-8` via its inherited history);
- tags `fluent-desktop-v0.1.1` and `fluent-web-v0.1.0`;
- `origin/main`, `origin/HEAD` and `origin/finalization/cycle-2`.

Therefore a normal clone of any of those refs still downloads the NLLB assets.
The Cycle 7 commit removed them from later trees but did not remove the old
objects.

## 2. Cleanup strategies

Do not treat any of these as legally cleared; legal review is required.

| Strategy | Effect | Trade-offs |
|---|---|---|
| Rewrite existing branches/tags and force-push | Removes assets from reachable history everywhere | Destructive, breaks clones/forks, requires coordinated force-push and remote authorization; **not performed** |
| Publish a sanitized mirror with rewritten history | Clean clone and clean tags under a new remote | New URLs/commit IDs; contributors must re-clone; original remote still has old objects unless deleted/archived |
| Publish a source snapshot/tarball from the clean HEAD | Clean source archive; no `.git` history | No Git history/bisect; GitHub repository history remains unclean unless the repo is restricted or replaced |
| Keep history and restrict distribution | No rewrite | Only viable if the retained assets are permitted, or the repository stays private; requires a decision outside engineering |

The safest engineering preparation is the sanitized mirror plus a clean source
snapshot. Neither was published.

## 3. Sanitized history candidate

A candidate was prepared in an isolated bare mirror, outside the working
repository:

```text
/home/neon/fluent-build/cycle8-history-candidate.git
```

Procedure (reproducible with available tools; `git-filter-repo` was not
installed and no package was added):

```bash
git clone --mirror --no-local /tmp/opencode/fluent-cycle2 candidate.git
cd candidate.git
git filter-branch \
  --index-filter 'git rm -r --cached --ignore-unmatch \
      models/nllb-200-1.3b models/nllb-200-3.3b' \
  --tag-name-filter cat -- --all
rm -rf refs/original
git reflog expire --expire=now --all
git gc --prune=now
```

Verification performed on the candidate:

- Current `finalization/cycle-7` tree hash is **identical** to the real
  `7f85db6` tree (`bb47664f…`), so no source file changed.
- Commit count is preserved (41 reachable commits at the time of the audit).
- `git rev-list --objects --all` contains **no** `models/nllb-200-*` path.
- Both tags were rewritten by `--tag-name-filter cat` and contain zero model
  files.
- `git fsck --full` is clean.
- Candidate pack size is ~405 KiB versus ~8.8 MB for the original repository.
- `main` differs from the original only by the 20 removed asset paths.

The candidate is **not** the working repository, was not pushed, and did not
alter any real ref. A final candidate should be regenerated from the Cycle 8
HEAD before any release process, then reviewed and authorized by the project
owner and legal counsel.

## 4. Source snapshot

A source snapshot is the lowest-risk alternative to publishing full Git
history. It is generated from the exact tracked HEAD with Git's own archive:

```bash
./scripts/make-source-snapshot.sh
# equivalent to:
# git archive --format=tar.gz -o dist/fluent-0.1.0-source.tar.gz HEAD
```

Properties:

- no `.git`, `target/`, `dist/`, local models or untracked files;
- only tracked source, documentation, licenses and `models/README.md`;
- deterministic for a given commit;
- adjacent SHA-256 file;
- not published or uploaded.

The snapshot is in `dist/` (ignored, local only). Its current checksum is
recorded in the Cycle 8 report and can be regenerated from the final HEAD.

## 5. What remains true

- Current tree: **clean of NLLB assets**.
- Git history: **retains them** reachable from main, tags and remote refs.
- Remote: **unchanged** — no push, tag change, force-push or release.
- Candidate: **prepared locally only**.
- Legal review: **not obtained**.
- Official NLLB weights: still CC-BY-NC-4.0, user-supplied, never bundled.

No claim of public-repository cleanliness is made.
