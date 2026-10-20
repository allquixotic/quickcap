# Publishing signed builds

Source destination: `https://github.com/allquixotic/quickcap`.
Local builds are unsigned; public tag releases must pass Azure signing and
Authenticode verification. CI never substitutes an unsigned release when signing
is unavailable. No workflow installs or launches the application persistently.

Create an Azure Artifact Signing account (formerly Trusted Signing) and a public
trust certificate profile after identity validation. Assign **Artifact Signing
Certificate Profile Signer** to an Entra application, scoped to that profile.
Configure a federated OIDC credential for the GitHub `release` environment:

```text
issuer: https://token.actions.githubusercontent.com
subject: repo:allquixotic/quickcap:environment:release
audience: api://AzureADTokenExchange
```

In the GitHub repository, create environment `release` and restrict it to release
tags. Store these environment secrets:

| Secret | Value |
| --- | --- |
| `AZURE_CLIENT_ID` | Entra application/client ID |
| `AZURE_TENANT_ID` | Tenant ID |
| `AZURE_SUBSCRIPTION_ID` | Azure subscription ID |

Store these environment variables:

| Variable | Value |
| --- | --- |
| `AZURE_SIGNING_ENDPOINT` | Regional signing endpoint, e.g. `https://eus.codesigning.azure.net/` |
| `AZURE_SIGNING_ACCOUNT` | Artifact Signing account name |
| `AZURE_CERTIFICATE_PROFILE` | Public certificate profile name |

The workflow uses GitHub OIDC and Azure CLI credentials; no long-lived client
secret is stored. Third-party actions are pinned to full commits. Regular pull
requests have read-only permissions and cannot access the signing environment.

After CI and a desktop smoke test pass, bump `Cargo.toml` and `Cargo.lock`, commit,
and push a matching `vX.Y.Z` tag. The release job checks tag/package agreement,
signs the single executable with an RFC 3161 timestamp, checks Authenticode status,
calculates SHA-256 **after signing**, and attaches `quickcap.exe` and `SHA256SUMS`
to the GitHub release. A zip or adjacent DLLs are unnecessary.

Official references: [Azure signing action](https://github.com/Azure/artifact-signing-action),
[OpenID Connect setup](https://github.com/Azure/login#login-with-openid-connect-oidc-recommended),
[Media Foundation MP4 support](https://learn.microsoft.com/en-us/windows/win32/medfound/mpeg-4-file-sink).
