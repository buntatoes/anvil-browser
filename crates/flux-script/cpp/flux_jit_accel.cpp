// Flux Browser Native C++20 JavaScript JIT Bytecode & Arithmetic Kernel
// Author: Buntos
// License: Apache-2.0

using u32 = unsigned int;
using i64 = long long;

extern "C" {

u32 flux_cpp_jit_kernel_version() noexcept {
    return 15u;
}

// Opcode 0: Add, 1: Sub, 2: Mul, 3: Div, 4: Mod
double flux_cpp_jit_eval_binary(u32 opcode, double lhs, double rhs) noexcept {
    switch (opcode) {
        case 0u:
            return lhs + rhs;
        case 1u:
            return lhs - rhs;
        case 2u:
            return lhs * rhs;
        case 3u:
            return rhs == 0.0 ? 0.0 : (lhs / rhs);
        case 4u: {
            if (rhs == 0.0) {
                return 0.0;
            }
            const i64 l_int = static_cast<i64>(lhs);
            const i64 r_int = static_cast<i64>(rhs);
            return r_int == 0 ? 0.0 : static_cast<double>(l_int % r_int);
        }
        default:
            return 0.0;
    }
}

// Fast native C++20 loop reduction for counted arithmetic loops
i64 flux_cpp_jit_sum_range_i64(i64 start, i64 end_exclusive, i64 step) noexcept {
    if (step <= 0 || start >= end_exclusive) {
        return 0;
    }
    i64 acc = 0;
    i64 iters = 0;
    for (i64 i = start; i < end_exclusive && iters < 100000; i += step, ++iters) {
        acc += i;
    }
    return acc;
}

} // extern "C"
