//! Where the camera actually sits, and what that means for what gets drawn.
//!
//! The renderer needs no idea any of this exists. [`Camera`] has no look
//! target — the view matrix is built from `position` plus `yaw`/`pitch` alone
//! — so a third-person camera is *only* a different position, computed after
//! movement and before the frustum is rebuilt. The controllers keep owning
//! orientation; this module owns placement.
//!
//! The pull-in is the part worth being careful about. Backing into a wall
//! must slide the camera forward rather than push it through the rock, and a
//! pivot that is itself inside a block (standing in a doorway as it closes,
//! say) reports an obstruction at distance zero — without a floor on the
//! distance the camera would collapse onto the player's own head and render
//! the inside of their skull.

use glam::{DVec3, Vec3};
use vx_render::Camera;
use vx_world::World;

/// How far behind the pivot the follow camera sits when nothing is in the way.
pub const THIRD_PERSON_DISTANCE: f32 = 4.2;
/// How far above the eye pivot the orbit is anchored.
pub const THIRD_PERSON_LIFT: f32 = 0.35;
/// The camera never comes closer to the pivot than this, whatever the terrain.
pub const MIN_ORBIT_DISTANCE: f32 = 0.6;
/// Clearance kept between the camera and a wall it has pulled in against.
pub const CAMERA_SKIN: f32 = 0.25;

/// Whose eyes the player is looking through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// Down the player's own nose.
    #[default]
    FirstPerson,
    /// Over the player's shoulder.
    ThirdPerson,
    /// Through a machine's camera; the body is elsewhere.
    Fpv,
}

impl ViewMode {
    /// The toggle. A feed is left alone — you leave it by hanging up, not by
    /// pressing the camera key.
    pub fn cycled(self) -> Self {
        match self {
            ViewMode::FirstPerson => ViewMode::ThirdPerson,
            ViewMode::ThirdPerson => ViewMode::FirstPerson,
            ViewMode::Fpv => ViewMode::Fpv,
        }
    }

    /// Should the player's body be drawn? Only when they can see it — first
    /// person would put geometry inside the near plane, and a machine's feed
    /// is looking somewhere else entirely.
    pub fn draws_body(self) -> bool {
        matches!(self, ViewMode::ThirdPerson)
    }

    /// Should the handheld drill be drawn? Only when it is in your hands.
    pub fn draws_viewmodel(self) -> bool {
        matches!(self, ViewMode::FirstPerson)
    }
}

/// How far back the orbit may sit before terrain gets in the way, or `None`
/// when there is not even [`MIN_ORBIT_DISTANCE`] of clear air behind the eye.
///
/// `back` is the unit direction from the pivot toward the camera.
///
/// The `None` is stage 47's correction. This used to `clamp` a blocked
/// distance back *up* to the floor, so a wall closer than
/// `MIN_ORBIT_DISTANCE + CAMERA_SKIN` put the camera up to a quarter of a
/// block *inside solid rock* — the floor was there to stop the orbit
/// collapsing onto the body, and it was quietly also overruling the wall.
/// Refusing is the honest answer, and the caller has somewhere sensible to
/// go with it.
pub fn clear_orbit_distance(world: &World, pivot: DVec3, back: Vec3, wanted: f32) -> Option<f32> {
    let blocked = vx_world::raycast_solid(world, world.registry(), pivot, back, wanted)
        .map(|hit| hit.distance - CAMERA_SKIN)
        .unwrap_or(wanted);
    (blocked >= MIN_ORBIT_DISTANCE).then(|| blocked.min(wanted))
}

/// The camera's position for this frame, given where its owner's eyes are.
///
/// `camera` supplies orientation only; the returned position replaces its own.
pub fn camera_placement(world: &World, camera: &Camera, pivot: DVec3, mode: ViewMode) -> DVec3 {
    match mode {
        // A feed's position is set by whatever it is a feed of.
        ViewMode::FirstPerson | ViewMode::Fpv => pivot,
        ViewMode::ThirdPerson => {
            let anchor = pivot + DVec3::Y * THIRD_PERSON_LIFT as f64;
            let back = -camera.forward();
            // Nowhere clear to stand off to: fall back to the eye for this
            // frame rather than putting the camera inside the wall. Backing
            // into a corner briefly becomes first person, which is what the
            // rough-edge note about filling the frame with your own back
            // wanted anyway.
            match clear_orbit_distance(world, anchor, back, THIRD_PERSON_DISTANCE) {
                Some(distance) => anchor + (back * distance).as_dvec3(),
                None => pivot,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::{BlockPos, ChunkPos};

    /// A world with real terrain around the origin.
    fn world() -> World {
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(0, 0), 1);
        world
    }

    fn camera_looking(yaw: f32, pitch: f32) -> Camera {
        Camera { yaw, pitch, ..Camera::default() }
    }

    /// A pivot well clear of the village buildings, in open air.
    fn open_pivot(world: &World) -> DVec3 {
        let ground = world.surface_y(0, 0).expect("origin loaded");
        DVec3::new(0.5, ground as f64 + 6.0, 0.5)
    }

    #[test]
    fn first_person_puts_the_camera_exactly_on_the_pivot() {
        let world = world();
        let camera = camera_looking(0.7, -0.2);
        let pivot = open_pivot(&world);
        assert_eq!(
            camera_placement(&world, &camera, pivot, ViewMode::FirstPerson),
            pivot
        );
        // A feed's position comes from the machine, so placement passes it
        // straight through too.
        assert_eq!(camera_placement(&world, &camera, pivot, ViewMode::Fpv), pivot);
    }

    #[test]
    fn third_person_sits_behind_and_above_the_pivot() {
        let world = world();
        let camera = camera_looking(0.0, 0.0);
        let pivot = open_pivot(&world);

        let placed = camera_placement(&world, &camera, pivot, ViewMode::ThirdPerson);
        let anchor = pivot + DVec3::Y * THIRD_PERSON_LIFT as f64;
        let back = -camera.forward();

        // Directly along -forward from the lifted anchor, at full distance in
        // open air.
        let offset = (placed - anchor).as_vec3();
        assert!(
            (offset.length() - THIRD_PERSON_DISTANCE).abs() < 1e-3,
            "sat {} away, wanted {THIRD_PERSON_DISTANCE}",
            offset.length()
        );
        assert!(
            offset.normalize().dot(back) > 0.999,
            "camera is not behind the pivot"
        );
        assert!(placed.y > pivot.y - THIRD_PERSON_DISTANCE as f64, "sank below the pivot");
    }

    #[test]
    fn the_orbit_camera_pulls_in_when_a_wall_is_behind_you() {
        let mut world = world();
        let ground = world.surface_y(0, 0).unwrap();
        let pivot = DVec3::new(0.5, ground as f64 + 2.0, 0.5);
        let stone = world.registry().id_of("engine:stone").unwrap();

        let camera = camera_looking(0.0, 0.0); // looking -Z, so the camera sits +Z
        let open = camera_placement(&world, &camera, pivot, ViewMode::ThirdPerson);

        // Drop a wall two blocks behind the pivot.
        for dy in -1..=2 {
            for dx in -1..=1 {
                world.set_block(
                    BlockPos::new(dx, ground + 2 + dy, 2),
                    stone,
                );
            }
        }
        let blocked = camera_placement(&world, &camera, pivot, ViewMode::ThirdPerson);

        assert!(
            (blocked - pivot).length() < (open - pivot).length(),
            "the wall did not pull the camera in"
        );
        assert!(
            !world.is_solid(BlockPos::new(
                blocked.x.floor() as i32,
                blocked.y.floor() as i32,
                blocked.z.floor() as i32
            )),
            "the camera ended up inside the wall at {blocked:?}"
        );
    }

    #[test]
    fn a_buried_pivot_falls_back_to_the_eye_rather_than_into_the_rock() {
        // A pivot inside solid rock makes the raycast report an obstruction
        // at distance zero. The floor used to *clamp* that back up, which put
        // the camera a quarter of a block inside the wall — the floor was
        // there to stop the orbit collapsing onto the body and was quietly
        // also overruling the geometry. Refusing is the honest answer: the
        // frame is first person, and first person is never inside anything.
        let world = world();
        let ground = world.surface_y(0, 0).unwrap();
        let buried = DVec3::new(0.5, ground as f64 - 4.0, 0.5);
        let camera = camera_looking(1.1, 0.0);
        let anchor = buried + DVec3::Y * THIRD_PERSON_LIFT as f64;

        assert_eq!(
            clear_orbit_distance(&world, anchor, -camera.forward(), THIRD_PERSON_DISTANCE),
            None,
            "a buried anchor reported clear air behind it"
        );
        let placed = camera_placement(&world, &camera, buried, ViewMode::ThirdPerson);
        assert_eq!(placed, buried, "a blocked orbit did not fall back to the eye");
    }

    #[test]
    fn the_camera_keeps_clear_of_geometry_all_the_way_round() {
        // Sweeping the view in a real world is the check that matters: any
        // yaw where the camera ends up inside a hillside is a frame of
        // looking at the inside of the ground.
        let world = world();
        let ground = world.surface_y(0, 0).unwrap();
        let pivot = DVec3::new(0.5, ground as f64 + 1.6, 0.5);

        for step in 0..64 {
            let yaw = step as f32 * std::f32::consts::TAU / 64.0;
            for pitch in [-0.6f32, 0.0, 0.6] {
                let camera = camera_looking(yaw, pitch);
                let placed = camera_placement(&world, &camera, pivot, ViewMode::ThirdPerson);
                let cell = BlockPos::new(
                    placed.x.floor() as i32,
                    placed.y.floor() as i32,
                    placed.z.floor() as i32,
                );
                assert!(
                    !world.is_solid(cell),
                    "camera inside geometry at yaw {yaw}, pitch {pitch}: {placed:?}"
                );
            }
        }
    }

    /// The margin, not the cell. `the_camera_keeps_clear_of_geometry_all_the
    /// _way_round` asks whether the camera's own block is solid, which a
    /// camera sitting a centimetre off a wall face passes just as happily as
    /// one sitting a metre off it. `CAMERA_SKIN` is the promise: a quarter of
    /// a block of air between the lens and whatever it pulled in against.
    /// Sweeping the yaw inside a tight room is where the old clamp used to
    /// break it — the floor overruled the wall and put the lens inside the
    /// stone — so that is where the promise gets checked.
    #[test]
    fn the_orbit_camera_keeps_its_whole_skin_off_every_wall() {
        let mut world = world();
        let ground = world.surface_y(0, 0).unwrap();
        let stone = world.registry().id_of("engine:stone").unwrap();
        let floor = ground + 1;

        // A closed room three blocks across, so every yaw finds a wall well
        // inside THIRD_PERSON_DISTANCE.
        for x in -3i32..=3 {
            for z in -3i32..=3 {
                for y in floor..=floor + 4 {
                    let shell = x.abs() == 3 || z.abs() == 3 || y == floor || y == floor + 4;
                    world.set_block(
                        BlockPos::new(x, y, z),
                        if shell { stone } else { vx_core::BlockId::AIR },
                    );
                }
            }
        }

        let pivot = DVec3::new(0.5, floor as f64 + 1.6, 0.5);
        let mut pulled_in = 0;
        for step in 0..64 {
            let yaw = step as f32 * std::f32::consts::TAU / 64.0;
            for pitch in [-0.4f32, 0.0, 0.4] {
                let camera = camera_looking(yaw, pitch);
                let anchor = pivot + DVec3::Y * THIRD_PERSON_LIFT as f64;
                let back = -camera.forward();
                match clear_orbit_distance(&world, anchor, back, THIRD_PERSON_DISTANCE) {
                    Some(distance) => {
                        pulled_in += 1;
                        // There is a whole skin of air past where the lens
                        // sits: a ray that far and a hair further finds
                        // nothing.
                        let reach = distance + CAMERA_SKIN - 1.0e-3;
                        assert!(
                            vx_world::raycast_solid(
                                &world,
                                world.registry(),
                                anchor,
                                back,
                                reach
                            )
                            .is_none(),
                            "only {distance} of clear air at yaw {yaw}, pitch {pitch}:                              the lens is inside its own skin"
                        );
                        assert!(
                            distance >= MIN_ORBIT_DISTANCE,
                            "orbited closer than the floor at yaw {yaw}"
                        );
                    }
                    None => {
                        let placed =
                            camera_placement(&world, &camera, pivot, ViewMode::ThirdPerson);
                        assert_eq!(placed, pivot, "a refused orbit did not fall back to the eye");
                    }
                }
            }
        }

        assert!(pulled_in > 0, "no yaw in a three-block room pulled the camera in");
    }

    #[test]
    fn cycling_the_view_returns_to_first_person() {
        assert_eq!(ViewMode::FirstPerson.cycled(), ViewMode::ThirdPerson);
        assert_eq!(ViewMode::FirstPerson.cycled().cycled(), ViewMode::FirstPerson);
        // A live feed is left alone by the camera key.
        assert_eq!(ViewMode::Fpv.cycled(), ViewMode::Fpv);
    }

    #[test]
    fn only_third_person_draws_the_body_and_only_first_person_the_viewmodel() {
        assert!(!ViewMode::FirstPerson.draws_body());
        assert!(ViewMode::ThirdPerson.draws_body());
        assert!(!ViewMode::Fpv.draws_body());

        assert!(ViewMode::FirstPerson.draws_viewmodel());
        assert!(!ViewMode::ThirdPerson.draws_viewmodel());
        assert!(!ViewMode::Fpv.draws_viewmodel());
    }
}
