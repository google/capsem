---
title: Build your own image
description: How do I build and run an OCI image with my team's tools in Capsem?
---

This guide shows how to package your team's tools in a Linux OCI image,
publish it to a registry, authorize it in Capsem policy, and run it in a VM.
You need a container builder,
permission to publish to your registry, and an administrator who can approve
the image.

## Create the image

Start with a Dockerfile in an empty build directory:

```dockerfile
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl git python3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 agent
USER agent
WORKDIR /workspace
CMD ["sleep", "infinity"]
```

Add your agent and dependencies during the build. Keep credentials out of
Dockerfile arguments, layers, and copied files. The default command must keep
running if you want to attach a terminal later. For repeatable builds, pin
the base image by digest and record dependency versions.

Capsem supplies the VM runtime. You do not need to rebuild its kernel,
initrd, or rootfs to add application tools.

## Build and publish

Choose `linux/arm64` for an Apple Silicon or arm64 host, `linux/amd64` for an
x86_64 host, or publish both platforms for a mixed fleet. Replace the registry
path with one you own. For an Apple Silicon host:

```sh
docker buildx build --platform linux/arm64 -t ghcr.io/your-org/your-image:v1 --push .
```

Record the registry's returned image digest; a tag can move.

## Admit the image

An administrator must allow the registry source and the images that may run.
For an owned registry namespace, merge this into the appropriate existing
`[images]` section:

```toml
[images]
sources = ["ghcr.io"]
admit = ["ghcr.io/your-org/**"]
```

Replace `your-org` with your namespace. This example admits that namespace;
use the narrowest selector appropriate to your deployment. Source permission
allows fetching; admission decides whether the resolved content may execute.
A corporate `[images]` section replaces the user's image policy, including
catalog selection. Preserve existing entries your team still needs.

Validate the containing file with `capsem-admin settings validate PATH --json`.
See [Configuration reference](/reference/configuration/) for precedence.

## Run and check

Once the policy is active, prefetch and create the VM. For a private registry,
add `--registry-user USER` before `--image` and supply
`CAPSEM_REGISTRY_PASSWORD` in the host environment. `--registry-ca PATH` adds a
PEM CA for that pull only.

```sh
capsem images pull ghcr.io/your-org/your-image:v1
capsem create -n custom --image ghcr.io/your-org/your-image:v1
capsem exec custom 'python3 --version'
capsem shell custom
```

Check that your tool runs under the intended user and that required
destinations are allowed. If a TLS client uses its own trust store, configure
it to trust the Capsem inspection CA; certificate pinning can prevent
inspection. See [HTTPS inspection](/architecture/security-engine/#protocol-parsing-and-https-inspection).

To distribute a modified VM runtime as well, use the
[enterprise deployment guide](/guides/enterprise/deploy-capsem/) and
[manifest contract](/reference/manifests/).
