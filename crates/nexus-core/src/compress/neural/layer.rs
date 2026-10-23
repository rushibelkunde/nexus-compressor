//! Fixed-point neural network layer with integer matrix operations (Layer 3).
//!
//! Provides deterministic feed-forward and online integer backpropagation
//! with zero floating-point arithmetic (ADR-003).

/// Fully-connected dense layer with 16-bit integer weights.
pub struct DenseLayer {
    pub in_dim: usize,
    pub out_dim: usize,
    pub weights: Vec<i16>,
    pub biases: Vec<i32>,
    pub last_inputs: Vec<i16>,
    pub last_pre_activations: Vec<i32>,
    pub outputs: Vec<i16>,
}

impl DenseLayer {
    /// Creates a new layer initialized with small non-zero integer weights.
    pub fn new(in_dim: usize, out_dim: usize) -> Self {
        let mut weights = vec![0i16; in_dim * out_dim];
        let mut rng: u64 = 0x5EED_C0DE_1234;

        // Initialize with small random weights in [-64, 64]
        for w in weights.iter_mut() {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            *w = (((rng >> 32) % 129) as i16) - 64;
        }

        Self {
            in_dim,
            out_dim,
            weights,
            biases: vec![0i32; out_dim],
            last_inputs: vec![0i16; in_dim],
            last_pre_activations: vec![0i32; out_dim],
            outputs: vec![0i16; out_dim],
        }
    }

    /// Forward pass computing integer activations with fixed-point LeakyReLU.
    ///
    /// Inputs are scaled by 256 (8-bit fractional scale).
    #[inline(always)]
    pub fn forward(&mut self, inputs: &[i16]) -> &[i16] {
        self.last_inputs.copy_from_slice(inputs);

        for j in 0..self.out_dim {
            let mut acc: i64 = self.biases[j] as i64;

            for (i, &x) in inputs.iter().enumerate().take(self.in_dim) {
                let w = self.weights[i * self.out_dim + j] as i64;
                acc += (w * x as i64) >> 8;
            }

            self.last_pre_activations[j] = acc.clamp(-2_000_000_000, 2_000_000_000) as i32;

            // Integer LeakyReLU activation: f(x) = x if x > 0 else x / 8
            let act = if acc > 0 {
                acc.clamp(-32767, 32767) as i16
            } else {
                (acc >> 3).clamp(-32767, 32767) as i16
            };

            self.outputs[j] = act;
        }

        &self.outputs
    }

    /// Sparse forward pass: activates only non-zero input indices.
    ///
    /// Accelerates forward inference from O(in_dim * out_dim) to O(k * out_dim).
    #[inline(always)]
    pub fn forward_sparse(&mut self, active_indices: &[usize], scale: i16) -> &[i16] {
        for j in 0..self.out_dim {
            let mut acc: i64 = self.biases[j] as i64;

            for &idx in active_indices {
                if idx < self.in_dim {
                    let w = self.weights[idx * self.out_dim + j] as i64;
                    acc += (w * scale as i64) >> 8;
                }
            }

            self.last_pre_activations[j] = acc.clamp(-2_000_000_000, 2_000_000_000) as i32;

            let act = if acc > 0 {
                acc.clamp(-32767, 32767) as i16
            } else {
                (acc >> 3).clamp(-32767, 32767) as i16
            };

            self.outputs[j] = act;
        }

        &self.outputs
    }

    /// Dense backpropagation updating all weights and biases with integer SGD.
    pub fn backward(&mut self, inputs: &[i16], grad_out: &[i32], lr_shift: u32) {
        for (j, &g) in grad_out.iter().enumerate().take(self.out_dim) {
            let pre_act = self.last_pre_activations[j];
            let d_act = if pre_act > 0 { g } else { g >> 3 };

            let delta_bias = d_act >> lr_shift;
            self.biases[j] = self.biases[j].saturating_add(delta_bias);

            for (i, &x) in inputs.iter().enumerate().take(self.in_dim) {
                let w_idx = i * self.out_dim + j;
                let delta_w = (((x as i64) * (d_act as i64)) >> (lr_shift + 8)) as i16;
                self.weights[w_idx] = self.weights[w_idx]
                    .saturating_add(delta_w)
                    .clamp(-16384, 16384);
            }
        }
    }

    /// Online backpropagation updating weights and biases with integer SGD.
    #[inline(always)]
    pub fn backward_sparse(
        &mut self,
        active_indices: &[usize],
        scale: i16,
        grad_out: &[i32],
        lr_shift: u32,
    ) {
        for (j, &g) in grad_out.iter().enumerate().take(self.out_dim) {
            let pre_act = self.last_pre_activations[j];
            // LeakyReLU derivative: 1 if pre_act > 0 else 1/8
            let d_act = if pre_act > 0 { g } else { g >> 3 };

            // Update bias
            let delta_bias = d_act >> lr_shift;
            self.biases[j] = self.biases[j].saturating_add(delta_bias);

            // Update weights for active features
            for &idx in active_indices {
                if idx < self.in_dim {
                    let w_idx = idx * self.out_dim + j;
                    let delta_w = (((scale as i64) * (d_act as i64)) >> (lr_shift + 8)) as i16;
                    self.weights[w_idx] =
                        (self.weights[w_idx].saturating_add(delta_w)).clamp(-16384, 16384);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dense_layer_forward() {
        let mut layer = DenseLayer::new(4, 2);
        let inputs = [256i16, -256i16, 128i16, 0i16];
        let out = layer.forward(&inputs);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn test_dense_layer_sparse_equivalence() {
        let mut layer = DenseLayer::new(8, 4);
        let active = [1, 3, 5];
        let scale = 256i16;

        let mut full_inputs = [0i16; 8];
        for &idx in &active {
            full_inputs[idx] = scale;
        }

        // Compute dense forward
        let dense_out = layer.forward(&full_inputs).to_vec();

        // Compute sparse forward
        let sparse_out = layer.forward_sparse(&active, scale).to_vec();

        assert_eq!(dense_out, sparse_out);
    }

    #[test]
    fn test_dense_layer_learning_convergence() {
        let mut layer = DenseLayer::new(4, 1);
        let active = [0, 2];
        let scale = 256i16;

        // Train network to produce positive output
        for _ in 0..100 {
            layer.forward_sparse(&active, scale);
            let current = layer.outputs[0] as i32;
            let target = 2000i32;
            let error = target - current;
            layer.backward_sparse(&active, scale, &[error], 6);
        }

        layer.forward_sparse(&active, scale);
        assert!(
            layer.outputs[0] > 1000,
            "Target output should increase after SGD updates, got {}",
            layer.outputs[0]
        );
    }
}
