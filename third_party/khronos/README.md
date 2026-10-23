# Khronos development source pool (CFA4)

These 14 repositories are pinned Git submodules **for future CFA4 features, modules and auxiliary applications**. They are not enabled as CFA4 runtime dependencies, Cargo workspace members, CMake build targets, CI jobs or release inputs by this change. The original FilmCraft project is not modified.

## Obtain sources selectively

From the CFA4 repository root, fetch one source when needed:

```sh
git submodule update --init -- third_party/khronos/Vulkan-Video-Samples
```

To fetch all 14 sources (only if needed), from the CFA4 repository root:

```sh
git submodule update --init
```

A normal `git clone` of CFA4 **does not** fetch the source trees of these submodules automatically. Run `git submodule update --init` only for the sources needed. Some upstream repositories have their own nested submodules; these are **not fetched** unless explicitly requested with `--recursive`.

## Management policy

- Each submodule is pinned to an exact upstream commit SHA by the CFA4 Git tree (mode `160000`). Inspect with `git submodule status` and `git ls-tree -r HEAD third_party/khronos/`.
- Keep Khronos sources isolated from CFA4's application code. Never edit or automatically push to upstream repositories.
- Integrate only individually selected source components after technical, license, security, build and regression review.
- Existing FFmpeg/GPU, Rust/Cargo and Qt6 integration paths remain unchanged. No new performance, codec or GPU capability is claimed by adding sources.
- License and NOTICE requirements differ by upstream repository and transitive dependencies; verify them when incorporating source into distributable CFA4 components.
- Future updates to pinned revisions are separate, reviewed CFA4 changes, not implicit upstream tracking.

## Source inventory

| Upstream repository | CFA4 development purpose |
| --- | --- |
| Vulkan-Headers | Vulkan API definitions |
| Vulkan-Loader | Vulkan loader reference |
| Vulkan-Tools | Vulkan diagnostics and tools |
| Vulkan-ValidationLayers | API/synchronization validation |
| Vulkan-Video-Samples | Vulkan Video decode/encode references |
| Vulkan-Utility-Libraries | Vulkan helper libraries |
| SPIRV-Headers | SPIR-V definitions |
| SPIRV-Tools | Shader validation/optimization |
| glslang | GLSL to SPIR-V toolchain |
| OpenCL-SDK | OpenCL development samples and SDK |
| OpenCL-Headers | OpenCL interface headers |
| OpenCL-ICD-Loader | OpenCL ICD loader |
| KTX-Software | GPU texture assets and tools |
| glTF | 3D exchange specification/reference material |

Source URLs are maintained in the repository-root `.gitmodules`. Exact checked-in revisions are maintained by the Git submodule links; avoid maintaining a competing version list.
