---
title: What Makes Capsem Unique
description: The design choices that set Capsem apart, each with where it is explained and how it is proven.
sidebar:
  order: 1
---

A running list of what Capsem does that a container runtime or a cloud sandbox
does not. Each entry links to the page that explains it.

## Your session outlives its image

A session's workload runs on a **copy-on-write layer the session owns**,
stacked above whichever image it runs. The image is the read-only lower layer;
everything the agent or you write to the root filesystem -- a login under
`~/.claude`, `apt install`, a global `npm` package, shell config -- lands in
the session's layer.

The layer belongs to the session, not to the image:

- a **named** session keeps it across stop, resume and host restarts;
- a **fork** carries a copy of it;
- **changing the image** (`capsem create --from SESSION --image NEWER`) swaps
  the layer underneath and keeps everything above it -- upgrade the agent
  image and keep your agent's login, history and tools;
- an **ephemeral** session discards it with the VM.

Docker discards a container's writes when you move to a new image; Capsem
keeps the work and swaps the image. See
[Custom Images: The Session Layer](/architecture/custom-images/#the-session-layer).

## One VM per session

Every session is its own virtual machine (Apple Virtualization.framework on
macOS, KVM on Linux), with a minimal read-only runtime and the workload in a
user namespace inside it -- not a container sharing the host kernel. See
[Hypervisor](/architecture/hypervisor/).

## Every request is policy-checked and recorded

All traffic leaves through Capsem's proxy, which applies the session's network
and model policy and records each request, model call and tool call in the
session ledger. See [MITM Proxy](/architecture/mitm-proxy/) and
[Session Telemetry](/architecture/session-telemetry/).

## Applications are ordinary OCI images

Agents, toolchains and internal tools are plain OCI images, run as the
session's workload, admitted by the administrator's image policy and pinned by
digest. See [Custom Images](/architecture/custom-images/).
