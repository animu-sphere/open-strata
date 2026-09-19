# Motion Connectors

[`animu-sphere/motion-connectors`](https://github.com/animu-sphere/motion-connectors)
is the reference workspace for live motion inputs. It owns device and protocol
connectivity, source-coordinate normalization, timing and streaming. It passes
motion values to [USD Motion Plugins](usd-motion-plugins.md); avatar semantics
remain with the VRM and MMD repositories.

> The repository currently contains documentation and an empty build scaffold.
> Its [capability matrix](https://github.com/animu-sphere/motion-connectors/blob/main/docs/reference/CAPABILITY_MATRIX.md)
> claims no implemented connector. The VMC, mocopi and VRChat OSC code still
> lives in `usd-vrm-plugins` until the shared motion core is available as an
> installed dependency.

## OpenStrata boundary

This project will exercise a workspace of optional native libraries and tools
whose members need different external SDKs. A VMC build should not require a
browser or OpenXR SDK. The planned source migration will also exercise
digest-pinned library dependencies across repositories and generated CI for
hardware-free connector fixtures.

The current empty scaffold exposed a v0.22.10 CI gap: a source workspace with
no members cannot pass the generated `--graph-only` step. This is tracked in the
[v0.23.0 report intake](../roadmap/downstream-report-intake.md), rather than
counted as a completed OpenStrata adoption.

## Related documentation

- [Repository and current status](https://github.com/animu-sphere/motion-connectors)
- [Workspace contract](https://github.com/animu-sphere/motion-connectors/blob/main/docs/architecture/WORKSPACE.md)
- [Project roadmap](https://github.com/animu-sphere/motion-connectors/blob/main/docs/roadmap/README.md)
- [Reference projects](README.md)
