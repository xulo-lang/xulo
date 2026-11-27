// Stage 1 bootstrap runtime.
//
// The stage 1 backend (`xuloc/src/backend.xulo`) lowers the generic AST to C, not
// to typed machine code: every expression has the same `XVal` shape and every
// user type is a field table. This runtime is what the generated C program
// links against. It is deliberately not the product runtime
// (`crates/xulo-runtime`); it exists so the bootstrap compiler can compile the
// bootstrap subset — including itself — on a host that has a C compiler.
//
// Values are leaked rather than collected. A bootstrap compiler runs once and
// exits, so this keeps the runtime small and predictable; it is not a memory
// model for user programs (those use the stage 0 runtime).
//
// Compile as part of the generated translation unit:
//     cc -O1 generated.c -o program
// `xuloc/runtime/runtime.c` has no `main` and no platform SDK dependency beyond libc.
//
// The implementation is split into `runtime_*.inc` modules and assembled here
// as a unity build. The generated C includes this file, so the modules share a
// translation unit and the build protocol stays one source file plus libc.
// Dependency order: values and containers, operators, rendering, method
// dispatch, then the `std` modules.

#include "runtime_internal.h"

#include "runtime_value.inc"   // allocation, UTF-8, constructors, containers
#include "runtime_ops.inc"     // arithmetic, comparison, `is`, nullish
#include "runtime_render.inc"  // formatting and `StringBuilder`
#include "runtime_methods.inc" // intrinsic/member dispatch
#include "runtime_fs.inc"      // std::fs / std::os / std::process
#include "runtime_io.inc"      // std::io readers and terminal probes
#include "runtime_json.inc"    // std::json
#include "runtime_path.inc"    // std::path
#include "runtime_math.inc"    // std::math
#include "runtime_time.inc"    // std::time
#include "runtime_process.inc" // std::process::exec
