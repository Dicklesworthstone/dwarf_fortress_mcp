//! Closed spatial selection shared by standalone and compound entity queries.
//!
//! The caller admits and verifies the complete immutable source and charges
//! `work_units_per_entity` before scanning. Every evaluation additionally checks
//! that caller's budget. Geometry remains advisory source data.

use dfmcp_core::{DfmcpError, ErrorCode, MapCoord, MapCuboid, Result};
use dfmcp_world::{EntityRecord, Value as WorldValue, WorldSnapshot};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) region: Region,
    pub(super) geometry: Geometry,
    pub(super) relation: Option<Relation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Region {
    min: [i32; 3],
    max: [i32; 3],
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Geometry {
    Point {
        field: String,
    },
    Cuboid {
        min_field: String,
        max_field: String,
    },
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Relation {
    #[default]
    Intersects,
    Within,
    Contains,
}

pub(super) struct Selection {
    region: MapCuboid,
    geometry: Geometry,
    relation: Relation,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Matched(MapCuboid),
    Outside,
    Unknown(&'static str),
}

pub(super) fn coord([x, y, z]: [i32; 3]) -> MapCoord {
    MapCoord::new(x, y, z)
}

fn coordinates(value: MapCoord) -> [i32; 3] {
    [value.x, value.y, value.z]
}

impl Request {
    pub(super) fn normalize(self) -> Result<Selection> {
        super::super::core_query::bounded_names(self.geometry.fields(), 2)?;
        if matches!(&self.geometry, Geometry::Cuboid { min_field, max_field } if min_field == max_field)
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "cuboid geometry requires two distinct coordinate fields",
            ));
        }
        Ok(Selection {
            region: MapCuboid::new(coord(self.region.min), coord(self.region.max))?,
            geometry: self.geometry,
            relation: self.relation.unwrap_or_default(),
        })
    }
}

impl Geometry {
    fn fields(&self) -> Vec<String> {
        match self {
            Self::Point { field } => vec![field.clone()],
            Self::Cuboid {
                min_field,
                max_field,
            } => vec![min_field.clone(), max_field.clone()],
        }
    }

    fn json(&self) -> Value {
        match self {
            Self::Point { field } => json!({"kind":"point","field":field}),
            Self::Cuboid {
                min_field,
                max_field,
            } => json!({"kind":"cuboid","min_field":min_field,"max_field":max_field}),
        }
    }

    fn read(
        &self,
        entity: &EntityRecord,
        snapshot: &WorldSnapshot,
    ) -> std::result::Result<MapCuboid, &'static str> {
        let read = |name: &str| {
            let fact = entity.fields.get(name).ok_or("missing_field")?;
            let value = fact
                .known_value()
                .ok_or("unavailable_or_contradictory_field")?;
            if fact.observed_at > snapshot.tick {
                return Err("future_geometry");
            }
            match value {
                WorldValue::Coord(value) => Ok(*value),
                _ => Err("non_coordinate_field"),
            }
        };
        let (min, max) = match self {
            Self::Point { field } => {
                let point = read(field)?;
                (point, point)
            }
            Self::Cuboid {
                min_field,
                max_field,
            } => (read(min_field)?, read(max_field)?),
        };
        MapCuboid::new(min, max).map_err(|_| "inverted_cuboid")
    }
}

impl Relation {
    fn name(self) -> &'static str {
        match self {
            Self::Intersects => "intersects",
            Self::Within => "within",
            Self::Contains => "contains",
        }
    }

    fn matches(self, geometry: MapCuboid, region: MapCuboid) -> bool {
        match self {
            Self::Within => region.contains_cuboid(geometry),
            Self::Contains => geometry.contains_cuboid(region),
            Self::Intersects => {
                geometry.min.x <= region.max.x
                    && geometry.max.x >= region.min.x
                    && geometry.min.y <= region.max.y
                    && geometry.max.y >= region.min.y
                    && geometry.min.z <= region.max.z
                    && geometry.max.z >= region.min.z
            }
        }
    }
}

impl Selection {
    /// This is the original `sq1` spatial selector encoding. Callers choose
    /// their own domain separator and optional-selector marker around it.
    pub(super) fn append_identity(&self, bytes: &mut Vec<u8>) {
        fn string(bytes: &mut Vec<u8>, value: &str) {
            bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }
        for point in [self.region.min, self.region.max] {
            for dimension in coordinates(point) {
                bytes.extend_from_slice(&dimension.to_be_bytes());
            }
        }
        bytes.push(match &self.geometry {
            Geometry::Point { .. } => 0,
            Geometry::Cuboid { .. } => 1,
        });
        for field in self.geometry.fields() {
            string(bytes, &field);
        }
        string(bytes, self.relation.name());
    }

    pub(super) fn json(&self) -> Value {
        json!({
            "region":{"min":coordinates(self.region.min),"max":coordinates(self.region.max)},
            "geometry":self.geometry.json(), "relation":self.relation.name()
        })
    }

    pub(super) fn geometry_fields(&self) -> Vec<String> {
        self.geometry.fields()
    }

    /// At most two coordinate reads and one inclusive relation comparison;
    /// independent of the region's volume and the source population.
    pub(super) fn work_units_per_entity(&self) -> usize {
        3
    }

    pub(super) fn evaluate(
        &self,
        entity: &EntityRecord,
        snapshot: &WorldSnapshot,
        check: &mut dyn FnMut() -> Result<()>,
    ) -> Result<Decision> {
        check()?;
        Ok(match self.geometry.read(entity, snapshot) {
            Ok(geometry) if self.relation.matches(geometry, self.region) => {
                Decision::Matched(geometry)
            }
            Ok(_) => Decision::Outside,
            Err(reason) => Decision::Unknown(reason),
        })
    }

    pub(super) fn witness(&self, entity: &EntityRecord, geometry: MapCuboid) -> Value {
        json!({
            "min":coordinates(geometry.min), "max":coordinates(geometry.max),
            "facts":super::super::core_query::entity_json(entity, &self.geometry.fields())["fields"],
            "eligible_as_mutation_evidence":false
        })
    }
}
