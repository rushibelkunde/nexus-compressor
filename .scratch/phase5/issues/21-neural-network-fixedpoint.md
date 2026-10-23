# 21: Fixed-Point Neural Layer & Integer Matrix Math

**What to build:** Implement fixed-point neural network layers in `crates/nexus-core/src/compress/neural/layer.rs`.
- 16-bit integer weights (`i16`) and 32-bit accumulators/biases (`i32`).
- Fixed-point activation functions: Integer ReLU / LeakyReLU (`if x > 0 { x } else { x >> 3 }`).
- Integer forward pass: `output[j] = (Σ input[i] * weight[i][j] + bias[j]) >> shift`.
- Online integer backpropagation: `weight[i][j] += (grad[j] * input[i]) >> lr_shift`.
- Strictly pure integer arithmetic — zero IEEE 754 floats (ADR-003).

**Blocked by:** Phase 4 complete

**Status:** ready-for-agent

- [ ] `compress/neural/layer.rs`: `DenseLayer` with fixed-point forward and backward pass
- [ ] Clamping of weights to prevent integer overflow
- [ ] Unit tests for forward computation, gradient descent convergence, and zero float math
