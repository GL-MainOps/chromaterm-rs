# CI/CD setup: GitLab (origin) + GitHub (mirror)

One-time setup so that pushing a `vX.Y.Z` tag builds and publishes releases on
both GitLab and GitHub.

```
git push origin vX.Y.Z
        │
        ▼
GitLab  (gitlab.com/mainops/chromaterm-rs)          .gitlab-ci.yml
  pipeline: version → lint/test → build ×4 → release (Package Registry + Release)
        │  push mirror (branches + tags)
        ▼
GitHub  (github.com/<you>/chromaterm-rs)            .github/workflows/release.yml
  workflow: version/check → build ×4 → release (GitHub Release)
```

**Only tags trigger pipelines.** Branch pushes, merge requests and pull
requests start nothing. Run `make check` locally before tagging.

No secrets or CI/CD variables are needed in either pipeline: GitLab uses the
built-in `CI_JOB_TOKEN`, and GitHub the built-in `GITHUB_TOKEN`. The only
credential is the token the GitLab mirror uses to push to GitHub.

---

## 1. GitLab (origin)

### 1.1 Runners
Jobs run in Docker images (`rust:1-bookworm`) and have no runner tags, so any
Linux Docker runner works.

- **gitlab.com instance runners**: *Settings → CI/CD → Runners* → make sure
  **"Enable instance runners for this project"** is on.
  - Free namespaces may need **identity verification** (phone/credit card)
    before instance runners pick up jobs (*User settings → Account*).
  - A release uses ~6 jobs of a few minutes each. Check the namespace's
    compute-minute quota (*Group → Settings → Usage quotas*).
- **Self-hosted runner** (alternative): `gitlab-runner register` with the
  `docker` executor. Privileged mode is **not** needed.

### 1.2 Project features
*Settings → General → Visibility, project features, permissions*:
- **CI/CD**: enabled.
- **Package registry**: enabled. The release job uploads the binaries to the
  Generic Package Registry and links them from the release.
- **Releases**: enabled.

### 1.3 Who may release (recommended)
*Settings → Repository → Protected tags* → add **`v*`**, "Allowed to create":
**Maintainers**. The job token acts with the permissions of whoever pushed the
tag. Uploading packages and creating releases needs **Developer** or higher;
protected tags need Maintainer.

### 1.4 Check the pipeline definition (optional)
*Build → Pipeline editor → Validate* (or `glab ci lint` with the GitLab CLI).

---

## 2. GitHub (mirror)

### 2.1 Create the repository
Create an **empty** repository on GitHub, e.g. `github.com/<you>/chromaterm-rs`,
with no README, license or `.gitignore` (the mirror overwrites it).

### 2.2 Token for the mirror
GitHub → *Settings → Developer settings → Personal access tokens →
Fine-grained tokens → Generate new token*:

| Field | Value |
|---|---|
| Repository access | **Only select repositories** → `chromaterm-rs` |
| Permissions → Contents | **Read and write** |
| Permissions → Workflows | **Read and write** (required: the mirror pushes `.github/workflows/*`; without it GitHub rejects the push with *"refusing to allow a Personal Access Token to create or update workflow … without `workflow` scope"*) |
| Permissions → Metadata | Read (automatic) |
| Expiration | Your choice. **Renew it before it expires**, or mirroring stops silently. |

(Classic token alternative: scopes `repo` + `workflow`.)

### 2.3 Configure the push mirror (in GitLab)
GitLab → *Settings → Repository → Mirroring repositories → Add new*:

| Field | Value |
|---|---|
| Git repository URL | `https://github.com/<you>/chromaterm-rs.git` |
| Mirror direction | **Push** |
| Authentication method | Username and Password |
| Username | your GitHub username |
| Password | the token from 2.2 |
| Keep divergent refs | off |
| Mirror only protected branches | **off** (simplest: everything, including tags, is mirrored) |

Click **Mirror repository**, then the 🔄 **Update now** button to do the first
sync. After that, GitLab pushes to GitHub automatically shortly after every
push, tags included.

### 2.4 GitHub Actions settings
GitHub repo → *Settings → Actions → General*:
- **Actions permissions**: "Allow all actions and reusable workflows". If you
  restrict them, allow at least `actions/*`, `Swatinem/rust-cache@*` and
  `softprops/action-gh-release@*`.
- **Workflow permissions**: the release job requests `contents: write` itself.
  If release creation fails with *403 Resource not accessible by integration*,
  set this to **"Read and write permissions"** (an organization policy may be
  capping it).

---

## 3. Releasing

```sh
make check                               # nothing else checks branches: verify first
# bump `version` in Cargo.toml, commit "chore(release): vX.Y.Z"
git tag -a vX.Y.Z -m "ct vX.Y.Z"
git push origin main                     # code (no pipeline)
git push origin vX.Y.Z                   # → GitLab pipeline → mirror → GitHub workflow
```

The pipeline fails early (`version` job) if the tag doesn't match
`Cargo.toml`'s version.

Results:
- GitLab: *Deploy → Releases* (assets link to *Deploy → Package registry → ct*).
- GitHub: *Releases* (binaries attached).

Re-running a failed release: fix the problem, then move the tag
(`git tag -d vX.Y.Z && git push origin :refs/tags/vX.Y.Z`, re-tag, push). Or
retry the failed job from the pipeline page if the cause was transient. On
GitHub, delete the partial release before re-pushing the tag.

## 4. Troubleshooting

| Symptom | Fix |
|---|---|
| GitLab pipeline stuck in *pending* | No runner: enable instance runners (1.1) or verify your account. |
| `release` job: 403 on upload or release | The tag pusher needs Developer+; enable Package registry/Releases (1.2). |
| GitHub workflow didn't start | Mirror not updated: check *Mirroring repositories* for an error (expired token, missing **Workflows** permission). Click *Update now*. |
| GitHub `release` job: 403 | Workflow permissions → Read and write (2.4). |
| `version` job fails | Tag `vX.Y.Z` ≠ `version` in `Cargo.toml`. |
| `build (…-gnu)` fails installing zig | The runner needs internet access to PyPI (cargo-zigbuild/ziglang are pip-installed, pinned in `ci/build-release.sh`). |
