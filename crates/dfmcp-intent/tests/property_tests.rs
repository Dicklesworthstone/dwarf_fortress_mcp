#![forbid(unsafe_code)]

use dfmcp_core::{MapCoord, MapCuboid};
use dfmcp_intent::{Action, DigMode};
use std::error::Error;

struct SimplePrng(u64);

impl SimplePrng {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0xdead_beef_cafe_babe
        } else {
            seed
        })
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.0
    }

    fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    fn next_range(&mut self, min: i32, max: i32) -> i32 {
        let span = (max - min + 1) as u32;
        min + (self.next_u32() % span) as i32
    }
}

#[test]
fn test_property_cuboid_inversion_digest_invariance() -> Result<(), Box<dyn Error>> {
    let mut prng = SimplePrng::new(0x2026_0831_1111);

    for i in 0..100 {
        let x1 = prng.next_range(-100, 100);
        let y1 = prng.next_range(-100, 100);
        let z1 = prng.next_range(0, 50);

        let x2 = prng.next_range(-100, 100);
        let y2 = prng.next_range(-100, 100);
        let z2 = prng.next_range(0, 50);

        let c1 = MapCoord {
            x: x1,
            y: y1,
            z: z1,
        };
        let c2 = MapCoord {
            x: x2,
            y: y2,
            z: z2,
        };

        let cuboid_a = MapCuboid::from_corners(c1, c2);
        let cuboid_b = MapCuboid::from_corners(c2, c1);

        assert_eq!(cuboid_a, cuboid_b);

        let action_a = Action::DesignateDig {
            area: cuboid_a,
            mode: DigMode::Mine,
        };
        let action_b = Action::DesignateDig {
            area: cuboid_b,
            mode: DigMode::Mine,
        };

        assert_eq!(
            action_a.canonical_bytes(),
            action_b.canonical_bytes(),
            "Iteration {i}: inverted coordinate bounding boxes must produce byte-identical encodings"
        );
    }
    Ok(())
}

#[test]
fn test_property_labor_permutation_digest_invariance() {
    let mut prng = SimplePrng::new(0x2026_0831_2222);

    for i in 0..100 {
        let count = (prng.next_u32() % 10 + 2) as usize;
        let mut raw_ids: Vec<dfmcp_core::EntityId> = (0..count)
            .map(|_| dfmcp_core::EntityId::new((prng.next_u32() % 50 + 1) as u64))
            .collect();

        let action1 = Action::SetLabor {
            units: raw_ids.clone(),
            labor: "HAUL_STONE".to_string(),
            enabled: true,
        };

        // Permute and duplicate
        raw_ids.reverse();
        raw_ids.push(raw_ids[0]);

        let action2 = Action::SetLabor {
            units: raw_ids,
            labor: "HAUL_STONE".to_string(),
            enabled: true,
        };

        assert_eq!(
            action1.normalized(),
            action2.normalized(),
            "Iteration {i}: permuted/duplicated unit lists must normalize identically"
        );
        assert_eq!(
            action1.canonical_bytes(),
            action2.canonical_bytes(),
            "Iteration {i}: permuted/duplicated unit lists must produce byte-identical encodings"
        );
    }
}
