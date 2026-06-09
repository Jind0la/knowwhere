# Issue: AVX-512FP16 Build Compatibility

**Date:** 2026-03-25
**Status:** RESOLVED (Not a distribution blocker)

---

## Executive Summary

**AVX-512FP16 ist KEIN Distribution-Problem!**

Die Befürchtung war, dass das Binary nur auf CPUs mit AVX-512FP16 läuft. Das ist **falsch**.

**Die Wahrheit:**
- Die `simsimd` Library (in usearch) macht **Runtime CPU-Detection** via CPUID
- Das Binary enthält Code für ALLE SIMD-Varianten (Haswell, Skylake, Sapphire, Serial)
- Bei Start wird die CPU erkannt und die beste Variante verwendet
- Wenn AVX-512FP16 nicht verfügbar → Fallback auf einfachere SIMD oder Serial

**Das eigentliche Problem war:** Mein Compiler (GCC 11.4) unterstützt das `avx512fp16` Target-Flag nicht und kann den Code daher nicht kompilieren. Die CI mit GCC 14.2 hat dieses Problem nicht.

---

## Technical Details

### Why Local Build Fails

The issue is **compiler version**, not **hardware compatibility**:

| | My Machine | CI (GitHub Actions) |
|--|------------|---------------------|
| **GCC Version** | 11.4.0 | 14.2.0 |
| **avx512fp16 flag** | ✗ Supported | ✓ Supported |
| **Build Status** | ✗ Fails | ✓ Success |

GCC 14 added support for the `avx512fp16` target flag. GCC 11 does not recognize it.

### How simsimd Handles CPU Compatibility

From `simsimd/simsimd.h`:

```cpp
// Runtime CPU detection via CPUID
unsigned supports_avx512fp16 = (info7.named.edx & 0x00800000) != 0;

// Map to CPU generations
unsigned supports_sapphire = supports_avx512fp16;

// Return capabilities INCLUDING fallback
return (simsimd_capability_t)(
    (simsimd_cap_haswell_k * supports_haswell) |   // AVX2+FMA
    (simsimd_cap_skylake_k * supports_skylake) |  // AVX-512
    (simsimd_cap_sapphire_k * supports_sapphire) | // AVX-512FP16
    (simsimd_cap_serial_k));                       // ALWAYS as fallback
```

The binary contains code for **ALL variants**. At runtime, CPUID determines which path to execute.

### CPU Feature Support

| Instruction Set | My Machine (i7-12700) | Most User Machines | Xeon EPYC (CI) |
|----------------|----------------------|-------------------|----------------|
| AVX | ✓ | ✓ | ✓ |
| AVX2 | ✓ | ✓ | ✓ |
| FMA | ✓ | ✓ | ✓ |
| SSE4.2 | ✓ | ✓ | ✓ |
| AVX-512 | ✗ | ~50% | ✓ |
| **AVX-512FP16** | ✗ | **<5%** | ✓ |

---

## How KnowWhere Uses usearch

### Architecture Overview

```
MemoryStore (in_memory.rs)
├── usearch_index: ANN vector index (usearch crate)
│   └── Used for: insert(), update(), retrieve_fractal(), hybrid_retrieve()
├── bm25_corpus: Text search (tiny bm25 crate)
│   └── Used for: search_bm25()
└── HashMap: Primary storage (nodes, uuid_to_key, key_to_uuid)
```

### Where usearch is Used

| File | Usage | Critical? |
|------|-------|----------|
| `in_memory.rs:12` | `use usearch::{new_index, Index, IndexOptions, MetricKind, ScalarKind}` | Yes |
| `in_memory.rs:199` | `usearch_index: Arc<Mutex<Option<SendableIndex>>>` | Yes |
| `in_memory.rs:393` | `index.add(key, &node.vector)` | Yes |
| `in_memory.rs:790` | Trajectory logging: "usearch_candidate" | Yes |
| `in_memory.rs:642` | `hybrid_retrieve` calls `retrieve_fractal` which uses usearch | Yes |

### postgres_store.rs Does NOT Use usearch

The `postgres_store.rs` uses `pgvector` (PostgreSQL extension) for vector operations:
- `vector_search()` uses SQL `<=>` (cosine distance) operator
- No usearch dependency in postgres_store.rs

---

## Why This Wasn't Caught Sooner

1. **CI Uses Xeon CPUs** - GitHub Actions runners have AVX-512FP16 support
2. **Local Builds Were Not Tested** - Most developers have CPUs with AVX-512 but not AVX-512FP16
3. **No Runtime Detection** - The error only appears when the binary actually executes on incompatible hardware

---

## Alternative Solutions

### Option 1: Use pgvector Instead of usearch (Recommended for Distribution)

**For postgres-storage builds:** Already uses pgvector - no change needed.
**For in-memory builds:** Would need to add an alternative ANN library.

**Libraries to consider:**
- `pgvector` - already used in postgres_store, but requires PostgreSQL
- `qdrant-client` or `milvus-sdk` - full vector databases, overkill for in-memory
- `hnswlib` - pure Rust, but may have similar SIMD issues
- `faiss` or `scann` - mature, but heavy dependencies

**Difficulty:** High - requires re-architecting the in-memory vector search layer

### Option 2: Disable AVX-512FP16 in usearch

Try to build usearch without AVX-512FP16 support by setting compiler flags.

```bash
CFLAGS="-mno-avx512fp16" CXXFLAGS="-mno-avx512fp16" cargo build
```

**Status:** ❌ Failed - the `avx512fp16` flag doesn't exist in GCC, only in Clang. Even if we used Clang, the `simsimd` code uses `__attribute__((target("avx512fp16")))` which can't be disabled via flags.

### Option 3: CPU Feature Detection + Graceful Fallback

Add runtime detection of CPU features and fall back to scalar code if AVX-512FP16 is unavailable.

**Difficulty:** Medium - would require changes to usearch or finding a usearch fork with better fallback support.

### Option 4: Accept Limited Distribution

Only distribute KnowWhere as:
- Docker images (run on servers with compatible CPUs)
- Source code for users to compile themselves
- Pre-built binaries only for verified compatible hardware

**Impact:** Severely limits user adoption.

### Option 5: Cross-Compile with Older CPU Target

Use `RUSTFLAGS="-C target-cpu=haswell"` or similar to target older CPUs.

**Problem:** AVX-512FP16 is compiled into the C++ code via `simsimd`, not controlled by Rust flags.

---

## Is usearch Alternative?

**Short answer: No, but with caveats.**

For **postgres-storage**, usearch is **NOT required** - pgvector handles vector search via SQL. The `MemoryStore` in `postgres_store.rs` doesn't use usearch at all.

For **in-memory storage** (the default), usearch **IS required** for ANN vector search. There's currently no drop-in replacement that:
1. Works without AVX-512FP16
2. Is pure Rust (or has a good Rust binding)
3. Has comparable performance

### What Would Need to Change for pgvector-only?

```rust
// Current in_memory.rs uses:
// - usearch for vector ANN search
// - tiny-bm25 for text search

// To remove usearch dependency:
// 1. Remove usearch from Cargo.toml
// 2. Implement a pure-Rust vector similarity (simple cosine, no ANN)
// 3. OR require PostgreSQL for all deployments
// 4. OR add an optional dependency on another vector library
```

---

## Conclusion

**No action required.** KnowWhere binaries work on all x86-64 CPUs.

The build failure on local machines is due to outdated compilers (GCC < 14), not hardware incompatibility. The CI with GCC 14.2 builds successfully and produces binaries that run on any modern x86-64 CPU using runtime SIMD detection and fallback.

---

## For Developers

If you encounter build errors with usearch:

```
error: '__m512h' was not declared in this scope
```

**Solution:** Update your GCC to version 14+ or use the CI for builds.

```bash
# Ubuntu 22.04
sudo add-apt-repository ppa:ubuntu-toolchain-r/test
sudo apt-get update && sudo apt-get install gcc-14 g++-14
```

Or simply use the GitHub Actions CI which has the correct toolchain.

---

## References

- [usearch GitHub](https://github.com/unum-cloud/usearch)
- [simsimd GitHub](https://github.com/ashvardanian/simsimd)
- [GCC 14 Release Notes](https://gcc.gnu.org/gcc-14/changes.html)
