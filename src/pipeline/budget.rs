/// Nombre d'images d'un paquet qui tient dans 60 % de la mémoire libre.
/// `None` si l'accumulateur et une seule image dépassent déjà ce budget.
pub fn packet_len(free_bytes: usize, crop_h: i32, crop_w: i32, scale: i32, channels: usize) -> Option<usize> {
    if crop_h <= 0 || crop_w <= 0 || channels == 0 || free_bytes == 0 {
        return None;
    }
    let scale = scale.max(1) as usize;
    let crop = (crop_h as usize).saturating_mul(crop_w as usize);
    let out = crop.saturating_mul(scale).saturating_mul(scale);
    let accum = out.saturating_mul(channels).saturating_mul(4);
    let scratch = if channels >= 3 {
        crop.saturating_mul(3).saturating_mul(4).saturating_add(crop.saturating_mul(2))
    } else {
        0
    };
    let fixed = accum.saturating_add(scratch);
    let budget = (free_bytes as u128).saturating_mul(60) / 100;
    if (fixed as u128) >= budget {
        return None;
    }
    let per = crop.saturating_mul(4).max(1);
    let count = ((budget as usize) - fixed) / per;
    if count == 0 { None } else { Some(count) }
}

pub fn blend_passes(accum: &mut [f32], average: &[f32], weight: f32) {
    for (slot, value) in accum.iter_mut().zip(average) {
        *slot += *value * weight;
    }
}

pub fn scale_average(accum: &mut [f32], total_weight: f32) {
    let scale = 1.0 / total_weight.max(1.0e-8);
    for value in accum {
        *value *= scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_budget_refuses_a_packet() {
        assert!(packet_len(1000, 100, 100, 3, 3).is_none());
    }

    #[test]
    fn wide_budget_accepts_several_small_frames() {
        let count = packet_len(12 * 1024 * 1024 * 1024, 64, 64, 3, 3).unwrap();
        assert!(count > 10, "{count}");
    }

    #[test]
    fn blended_passes_match_one_weighted_average() {
        let mut accum = vec![0.0, 0.0];
        blend_passes(&mut accum, &[2.0, 0.0], 1.0);
        blend_passes(&mut accum, &[0.0, 4.0], 3.0);
        scale_average(&mut accum, 4.0);
        assert!((accum[0] - 0.5).abs() < 1e-5);
        assert!((accum[1] - 3.0).abs() < 1e-5);
    }
}
