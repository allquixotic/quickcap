# Publishing signed Windows builds

Public repository: [allquixotic/quickcap](https://github.com/allquixotic/quickcap).
The project builds and releases Windows x64 only. `quickcap.exe` is portable;
there is no installer or adjacent DLL bundle.

## Build and release flow

`ci.yml` runs on main pushes, pull requests, and manual dispatch. It installs
stable Rust on `windows-2025`, checks formatting and Clippy, runs the unit tests
and native software encoder tests, builds the executable and desktop test
drivers, and runs the executable's synthetic media smoke test. Rust dependencies
are cached. The unsigned executable is retained as a CI artifact for 14 days.

`release.yml` runs when a `v*` tag is pushed, or when manually dispatched against
an existing tag. It calls the same build workflow, then:

1. Checks that the tag matches `Cargo.toml` and its commit is on `main`.
2. Downloads that run's executable, then authenticates to Azure through GitHub OIDC.
3. Signs using Azure Artifact Signing with a SHA-256 digest and RFC 3161 timestamp.
4. Verifies Authenticode, the expected publisher, and the timestamp certificate.
5. Runs the synthetic media smoke test on the exact signed executable.
6. Computes SHA-256 after signing and creates GitHub build provenance.
7. Publishes `quickcap.exe`, `SHA256SUMS`, and
   `quickcap-attestation.sigstore.json` with generated release notes.

Failed tests, signing, publisher verification, or attestation stop publication.
There is no unsigned release fallback. All external Actions are pinned to full
commits. Pull requests receive read-only permissions and no signing environment.
Interactive tray and real desktop/audio tests are run separately on Windows 11;
GitHub's hosted Windows Server runner is build/test infrastructure, not an
additional supported application platform.

## Configured signing identity

The repository environment `azure-artifact-signing` permits only `v*` tags.
It uses the existing `allquixotic-signing` account in East US and its
`windows-public` certificate profile. The Entra application has only the
**Artifact Signing Certificate Profile Signer** role on that profile.

The repository-specific federated credential is `github-quickcap-release`:

```text
issuer: https://token.actions.githubusercontent.com
subject: repo:allquixotic@322644/quickcap@1427316603:environment:azure-artifact-signing
audience: api://AzureADTokenExchange
```

GitHub includes immutable owner and repository IDs in this repository's subject.
For a fork or replacement repository, read its actual `sub_claim_prefix` with
`gh api repos/OWNER/REPO/actions/oidc/customization/sub` before configuring trust.
Do not copy the quickcap subject to another repository.

These environment variables contain identifiers and policy, not credentials:

| Variable | Meaning |
| --- | --- |
| `AZURE_CLIENT_ID` | Entra application/client ID |
| `AZURE_TENANT_ID` | Tenant ID |
| `AZURE_SUBSCRIPTION_ID` | Azure subscription ID |
| `AZURE_SIGNING_ENDPOINT` | `https://eus.codesigning.azure.net/` |
| `AZURE_SIGNING_ACCOUNT` | `allquixotic-signing` |
| `AZURE_CERTIFICATE_PROFILE` | `windows-public` |
| `WINDOWS_PUBLISHER` | Exact approved certificate subject |

No client secret or signing private key is stored in GitHub. Signing keys stay
in Azure. Local developer builds are unsigned.

## Publish the next version

Update the package version in `Cargo.toml` and `Cargo.lock`, commit and push to
`main`, then wait for its Windows checks to pass. After a Windows 11 desktop
smoke test, push the matching tag:

```text
git tag -a vX.Y.Z -m "quickcap X.Y.Z"
git push origin vX.Y.Z
```

To retry a failed release, rerun its workflow against the same tag after fixing
external configuration. Code or workflow fixes should use a new version and tag
if a release has already been published.

Consumers can compare the executable against `SHA256SUMS`, inspect its Windows
Digital Signatures properties, and verify GitHub provenance:

```text
gh attestation verify quickcap.exe --repo allquixotic/quickcap
```

Official references: [Azure signing action](https://github.com/Azure/artifact-signing-action),
[OpenID Connect setup](https://github.com/Azure/login#login-with-openid-connect-oidc-recommended),
[GitHub OIDC subjects](https://docs.github.com/en/actions/reference/security/oidc),
[Media Foundation MP4 support](https://learn.microsoft.com/en-us/windows/win32/medfound/mpeg-4-file-sink).
