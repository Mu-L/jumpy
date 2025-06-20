use crate::prelude::*;

#[derive(HasSchema, Default, Debug, Clone)]
#[type_data(metadata_asset("sproinger"))]
#[repr(C)]
/// This is a sproinger
pub struct SproingerMeta {
    pub atlas: Handle<Atlas>,
    pub sound: Handle<AudioSource>,
    pub sound_volume: f64,
    pub body_size: Vec2,
    pub spring_velocity: f32,
}

pub fn game_plugin(game: &mut Game) {
    SproingerMeta::register_schema();
    game.init_shared_resource::<AssetServer>();
}

pub fn session_plugin(session: &mut SessionBuilder) {
    session
        .stages
        .add_system_to_stage(CoreStage::PreUpdate, hydrate)
        .add_system_to_stage(CoreStage::PostUpdate, update);
}

#[derive(Clone, Debug, HasSchema, Default)]
pub struct Sproinger {
    pub frame: u32,
    pub sproinging: bool,
}

fn hydrate(
    entities: Res<Entities>,
    mut hydrated: CompMut<MapElementHydrated>,
    element_handles: Comp<ElementHandle>,
    assets: Res<AssetServer>,
    mut sproingers: CompMut<Sproinger>,
    mut atlas_sprites: CompMut<AtlasSprite>,
    mut bodies: CompMut<KinematicBody>,
    mut nav_graph: ResMutInit<NavGraph>,
    transforms: Comp<Transform>,
    map: Res<LoadedMap>,
) {
    let mut not_hydrated_bitset = hydrated.bitset().clone();
    not_hydrated_bitset.bit_not();
    not_hydrated_bitset.bit_and(element_handles.bitset());

    let mut new_sproingers = Vec::new();
    for entity in entities.iter_with_bitset(&not_hydrated_bitset) {
        let element_handle = element_handles.get(entity).unwrap();
        let element_meta = assets.get(element_handle.0);

        if let Ok(SproingerMeta {
            atlas, body_size, ..
        }) = assets.get(element_meta.data).try_cast_ref()
        {
            new_sproingers.push(entity);
            hydrated.insert(entity, MapElementHydrated);
            atlas_sprites.insert(entity, AtlasSprite::new(*atlas));
            bodies.insert(
                entity,
                KinematicBody {
                    shape: ColliderShape::Rectangle { size: *body_size },
                    has_mass: false,
                    ..default()
                },
            );
            sproingers.insert(entity, sproinger::default());
        }
    }

    // Update the navigation graph with the new sproingers
    if !new_sproingers.is_empty() {
        let mut new_graph = nav_graph.as_ref().clone();

        for ent in new_sproingers {
            let pos = transforms.get(ent).unwrap().translation;
            let node = NavNode((pos.truncate() / map.tile_size).as_ivec2());
            let sproing_to = node.above().above().above().above().above().above();

            new_graph.add_edge(
                node,
                sproing_to,
                NavGraphEdge {
                    inputs: [PlayerControl::default()].into(),
                    distance: node.distance(&sproing_to),
                },
            );
        }
        **nav_graph = Arc::new(new_graph);
    }
}

fn update(
    entities: Res<Entities>,
    element_handles: Comp<ElementHandle>,
    assets: Res<AssetServer>,
    mut sproingers: CompMut<Sproinger>,
    mut atlas_sprites: CompMut<AtlasSprite>,
    mut bodies: CompMut<KinematicBody>,
    dynamic_bodies: Comp<DynamicBody>,
    mut collision_world: CollisionWorld,
    mut audio_center: ResMut<AudioCenter>,
) {
    for (entity, (sproinger, sprite)) in entities.iter_with((&mut sproingers, &mut atlas_sprites)) {
        let element_handle = element_handles.get(entity).unwrap();
        let element_meta = assets.get(element_handle.0);

        let asset = assets.get(element_meta.data);
        let Ok(SproingerMeta {
            sound,
            sound_volume,
            spring_velocity,
            ..
        }) = asset.try_cast_ref()
        else {
            unreachable!();
        };

        if sproinger.sproinging {
            match sproinger.frame {
                1 => sprite.index = 2,
                4 => sprite.index = 3,
                8 => sprite.index = 4,
                12 => sprite.index = 5,
                x if x >= 20 => {
                    sprite.index = 0;
                    sproinger.sproinging = false;
                    sproinger.frame = 0;
                }
                _ => (),
            }
            sproinger.frame += 1;
        }

        for collider_ent in collision_world.actor_collisions(entity) {
            if let Some(body) = bodies.get_mut(collider_ent) {
                let dynamic_body = dynamic_bodies.get(collider_ent);
                let is_dynamic = if let Some(dynamic_body) = dynamic_body {
                    dynamic_body.is_dynamic
                } else {
                    false
                };

                if !is_dynamic {
                    if body.velocity.y < *spring_velocity {
                        audio_center.play_sound(*sound, *sound_volume);
                        body.velocity.y = *spring_velocity;
                        sproinger.sproinging = true;
                    }
                } else {
                    let spring_velocity = *spring_velocity;

                    let _ = collision_world.mutate_rigidbody(
                        collider_ent,
                        |rb: &mut rapier::RigidBody| {
                            let mut vel = *rb.linvel();
                            if vel.y < spring_velocity {
                                vel.y = spring_velocity;
                                rb.set_linvel(vel, true);
                                audio_center.play_sound(*sound, *sound_volume);
                            }
                        },
                    );
                }
            }
        }
    }
}
