---
title: Deploy Capsem in your enterprise
description: How do I deploy Capsem with centrally managed images, policy, and reporting?
---

Distribute the native package and a corporate policy, then verify both allowed
and refused operations on a pilot host. You need device-management or package
installation authority and ownership of the policy distribution endpoint.

## Choose what your organization owns

| Need | Configure |
| --- | --- |
| Agent/tool dependencies | OCI images and image admission. |
| Access and detections | Corporate enforcement TOML and Sigma YAML. |
| Central constraints/reporting | `corp.toml`. |
| Private runtime distribution | Corporate release manifest and owned runtime artifacts. |

Start with the official runtime unless your organization needs to maintain
a different kernel/rootfs. Application tools belong in
[OCI images](/guides/enterprise/build-an-image/).

## Prepare corporate policy

Keep your corporate files together: `corp.toml`, enforcement TOML, and
detection YAML. Reference rule files relative to the corporate file:

```toml
[corp_rule_files]
enforcement = "enforcement.toml"
sigma = "detection.yaml"
```

Write constraints using the [enforcement guide](/guides/enterprise/enforce-security-policy/)
and [detection guide](/guides/enterprise/write-detection-rules/).
Validate before distributing:

```sh
capsem-admin settings validate corp.toml --json
capsem-admin enforcement validate enforcement.toml --source corp --json
capsem-admin detection validate detection.yaml --source corp --json
```

Deploy files through device management to `/etc/capsem/`, with administrator
ownership and permissions that prevent ordinary users changing them.
System corporate configuration takes precedence over user corporate files.
Do not rely on a user-editable file alone for an organization-enforced constraint.

## Install and provision a pilot

Install the appropriate signed native package using your existing management
system. Follow the [installation requirements](/getting-started/installation/),
including Linux KVM and Landlock ABI 6 or later.

For URL-provisioned corporate configuration:

```sh
capsem update --corp https://security.your-org.example/capsem/corp.toml
capsem status
```

Replace the example endpoint with yours. Provision referenced rule files at
the paths the corporate file resolves; pointing at a remote TOML file does not
by itself place arbitrary local rule files on the host.

Create a pilot VM. Confirm a permitted request succeeds, a corporate block
is refused, a user rule cannot override it, and the expected records appear.
An invalid corporate file must refuse security admission, not remove constraints.

## Use a private runtime channel when needed

Build the owned runtime through `capsem-admin image build`.
Generate its manifests, inventory, and evidence through the admin tools.
Then combine that runtime with selected official host packages using
`capsem-admin manifest corporate`.

Provide corporation/channel, official manifest, owned runtime manifest,
owned HTTPS runtime base, binary version, exact source commit, and output root.
Check the output with `capsem-admin manifest check`.
The [manifest reference](/reference/manifests/) defines those contracts.

Select the corporate asset manifest using
`capsem update --assets --manifest URL`. This locks the installation to its
corporate channel; test the update path on a pilot before fleet distribution.

## Connect observability and roll out

Configure your corporate reporting endpoints and verify delivery from the
pilot. Inspect both decision records and body completeness; endpoint
configuration alone is not delivery proof.

Record package/runtime versions, image digests, effective policy, and expected
allow/block tests for each rollout. Keep prior owned inputs available for your
recovery procedure. Do not assume changing channels or replacing a manifest
URL is a supported rollback after a corporate lock.

Use [Investigate a VM](/guides/enterprise/investigate-a-vm/) and the
[telemetry contract](/reference/telemetry/) for evidence collection.
