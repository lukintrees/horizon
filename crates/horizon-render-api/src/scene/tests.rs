use super::*;
use std::collections::HashSet;

#[derive(Default)]
struct Backend {
    next_mesh: u64,
    next_texture: u64,
    textures: std::rc::Rc<std::cell::RefCell<HashSet<TextureId>>>,
    fail_texture: Option<u64>,
    updates: Vec<([u32; 2], [u32; 2], Vec<u8>)>,
    fail_update: bool,
    meshes: HashSet<MeshId>,
    fail_remove: bool,
    origins: Vec<[f64; 3]>,
}
impl RenderBackend for Backend {
    type Error = &'static str;
    fn create_texture(&mut self, _: Image<'_>) -> Result<TextureId, Self::Error> {
        self.next_texture += 1;
        if self.fail_texture == Some(self.next_texture) {
            return Err("texture allocation failure");
        }
        let id = TextureId(self.next_texture);
        self.textures.borrow_mut().insert(id);
        Ok(id)
    }
    fn update_texture(
        &mut self,
        _: TextureId,
        at: [u32; 2],
        image: Image<'_>,
    ) -> Result<(), Self::Error> {
        if self.fail_update {
            return Err("update failure");
        }
        self.updates
            .push((at, [image.width, image.height], image.rgba.to_vec()));
        Ok(())
    }
    fn remove_texture(&mut self, id: TextureId) -> Result<(), Self::Error> {
        assert!(self.textures.borrow_mut().remove(&id));
        Ok(())
    }
    fn create_mesh(&mut self, _: &[crate::Vertex], _: &[u16]) -> Result<MeshId, Self::Error> {
        self.next_mesh += 1;
        let id = MeshId(self.next_mesh);
        self.meshes.insert(id);
        Ok(id)
    }
    fn create_entity_mesh(
        &mut self,
        _: &[crate::entity::LitVertex],
        _: &[u16],
    ) -> Result<MeshId, Self::Error> {
        Err("unused")
    }
    fn create_gui_mesh(
        &mut self,
        _: &[crate::gui::GuiVertex],
        _: &[u16],
    ) -> Result<MeshId, Self::Error> {
        Err("unused")
    }
    fn remove_mesh(&mut self, id: MeshId) -> Result<(), Self::Error> {
        if std::mem::take(&mut self.fail_remove) {
            return Err("cleanup failure");
        }
        assert!(self.meshes.remove(&id));
        Ok(())
    }
    fn render(&mut self, frame: &Frame<'_>) -> Result<(), Self::Error> {
        self.origins = frame.draws.iter().map(|draw| draw.origin).collect();
        Ok(())
    }
    fn read_rgba(&mut self, _: [u32; 4]) -> Result<Vec<u8>, Self::Error> {
        Err("unused")
    }
}
fn scene() -> WorldScene<Backend> {
    let atlas = crate::atlas::pack_atlas(
        &[crate::atlas::SpriteSize {
            width: 1,
            height: 1,
        }],
        4,
        1,
        1,
    )
    .unwrap();
    WorldScene::new(Backend::default(), atlas, &[&[255; 64]], &[255; 1024], 4).unwrap()
}
fn runs() -> [Run; 1] {
    [Run {
        page: 0,
        pass: Pass::Opaque,
        vertices: vec![],
        indices: vec![],
    }]
}
fn draw(scene: &mut WorldScene<Backend>, keys: &[SectionKey]) {
    scene
        .render(&SceneFrame {
            view_projection: crate::IDENTITY,
            camera: [0.; 3],
            clear: [0.; 4],
            fog: None,
            solid: keys,
            translucent: &[],
        })
        .unwrap();
}
#[test]
fn reset_validation_and_cleanup_failures_do_not_publish_a_new_origin() {
    let mut scene = scene();
    scene.replace_section([0; 3], &runs(), &[]).unwrap();
    scene.replace_section([5, 1, 0], &runs(), &[]).unwrap();
    assert_eq!(scene.reset_sections(-63), Err(SceneError::InvalidLayout));
    assert_eq!(scene.section_count(), 2);
    assert!(scene.unload_section([0; 3]).unwrap());
    scene.backend_mut().fail_remove = true;
    assert_eq!(
        scene.reset_sections(-64),
        Err(SceneError::Backend("cleanup failure"))
    );
    assert!(scene.has_section([5, 1, 0]));
    draw(&mut scene, &[[5, 1, 0]]);
    assert_eq!(scene.backend_mut().origins, [[80., 16., 0.]]);
    scene.reset_sections(-64).unwrap();
    assert_eq!(scene.section_count(), 0);
    scene.replace_section([0; 3], &runs(), &[]).unwrap();
    draw(&mut scene, &[[0; 3]]);
    assert_eq!(scene.backend_mut().origins, [[0., -64., 0.]]);
    assert_eq!(scene.backend_mut().meshes.len(), 1);
}

#[test]
fn enumerated_membership_tracks_replacement_unload_and_failed_reset() {
    let mut scene = scene();
    let membership = |scene: &WorldScene<Backend>| {
        let mut keys: Vec<_> = scene.section_keys().collect();
        keys.sort_unstable();
        keys
    };
    for key in [[2, 3, -1], [-2, 0, 3], [-2, 7, 3]] {
        scene.replace_section(key, &runs(), &[]).unwrap();
    }
    scene.replace_section([-2, 0, 3], &runs(), &[]).unwrap();
    assert_eq!(membership(&scene), [[-2, 0, 3], [-2, 7, 3], [2, 3, -1]]);
    scene.unload_column([-2, 3]).unwrap();
    assert_eq!(membership(&scene), [[2, 3, -1]]);
    scene.backend_mut().fail_remove = true;
    assert!(scene.reset_sections(-64).is_err());
    assert_eq!(membership(&scene), [[2, 3, -1]]);
    draw(&mut scene, &[[2, 3, -1]]);
    assert_eq!(scene.backend().origins, [[32., 48., -16.]]);
    scene.reset_sections(-64).unwrap();
    assert!(membership(&scene).is_empty());
    scene.replace_section([0; 3], &runs(), &[]).unwrap();
    assert_eq!(membership(&scene), [[0; 3]]);
    draw(&mut scene, &[[0; 3]]);
    assert_eq!(scene.backend().origins, [[0., -64., 0.]]);
    scene.unload_section([0; 3]).unwrap();
    assert!(membership(&scene).is_empty());
}
#[test]
fn column_removal_keeps_other_columns_and_retries_garbage() {
    let mut scene = scene();
    for key in [[-2, 0, 3], [-2, 23, 3], [1, 0, 3]] {
        scene.replace_section(key, &runs(), &[]).unwrap();
    }
    assert_eq!(scene.unload_column([-2, 3]), Ok(2));
    assert_eq!(scene.section_count(), 1);
    assert!(scene.has_section([1, 0, 3]));
    scene.backend_mut().fail_remove = true;
    assert_eq!(
        scene.unload_column([1, 3]),
        Err(SceneError::Backend("cleanup failure"))
    );
    assert!(scene.has_section([1, 0, 3]));
    assert_eq!(scene.unload_column([-2, 3]), Ok(0));
    assert_eq!(scene.backend_mut().meshes.len(), 1);
    assert_eq!(scene.unload_column([1, 3]), Ok(1));
    scene.collect_garbage().unwrap();
    assert!(scene.backend_mut().meshes.is_empty());
}

#[test]
fn paged_construction_uploads_lazily_and_reclaims_partial_resources_on_every_failure() {
    use crate::atlas::{AtlasError, SpriteSize, pack_atlas};
    let atlas = || {
        pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }; 2],
            4,
            1,
            2,
        )
        .unwrap()
    };
    for failure in 0..4 {
        let backend = Backend {
            fail_texture: (failure == 2).then_some(2),
            ..Backend::default()
        };
        let live = backend.textures.clone();
        let mut prepared = 0;
        let result = WorldScene::new_paged(backend, atlas(), &[255; 1024], 1, |index| {
            // The prior page was already uploaded, not accumulated in a Vec<Vec>.
            assert_eq!(live.borrow().len(), index);
            prepared += 1;
            if failure == 0 && index == 1 {
                return Err(AtlasError::OutOfMemory);
            }
            Ok(vec![255; if failure == 1 && index == 1 { 63 } else { 64 }])
        });
        if failure == 3 {
            assert_eq!(prepared, 2);
            assert_eq!(live.borrow().len(), 3); // two pages + lightmap
            drop(result.unwrap());
        } else {
            assert!(result.is_err());
        }
        assert!(live.borrow().is_empty());
    }
    let backend = Backend::default();
    let mut called = false;
    assert!(
        WorldScene::new_paged(backend, atlas(), &[], 1, |_| {
            called = true;
            Ok(vec![255; 64])
        })
        .is_err()
    );
    assert!(!called); // invalid lightmap is rejected before page preparation/upload
}

#[test]
fn sprite_updates_extrude_padding_without_replacing_resident_meshes_or_neighbours() {
    let mut scene = scene();
    scene.replace_section([0; 3], &runs(), &[]).unwrap();
    let meshes = scene.backend_mut().meshes.clone();
    scene.update_sprite(0, &[1, 2, 3, 4], 36).unwrap();
    let update = &scene.backend_mut().updates[0];
    assert_eq!(update.0, [0, 0]);
    assert_eq!(update.1, [3, 3]);
    assert_eq!(update.2, [1, 2, 3, 4].repeat(9));
    assert!(scene.update_sprite(1, &[0; 4], 36).is_err());
    assert!(scene.update_sprite(0, &[0; 3], 36).is_err());
    assert!(scene.update_sprite(0, &[0; 4], 35).is_err());
    assert_eq!(scene.backend_mut().updates.len(), 1);
    scene.backend_mut().fail_update = true;
    assert!(scene.update_sprite(0, &[0; 4], 36).is_err());
    assert_eq!(scene.backend_mut().meshes, meshes);
    assert_eq!(scene.section_count(), 1);
}

#[test]
fn paged_checkpoints_cancel_at_every_boundary_and_reclaim_partial_textures() {
    let atlas = crate::atlas::pack_atlas(
        &[crate::atlas::SpriteSize {
            width: 1,
            height: 1,
        }; 3],
        4,
        1,
        3,
    )
    .unwrap();
    assert_eq!(atlas.page_count(), 3);
    for stop in 0..=3 {
        let backend = Backend::default();
        let textures = backend.textures.clone();
        let mut seen = Vec::new();
        let result = WorldScene::new_paged_checked(
            backend,
            atlas.clone(),
            &[255; 1024],
            1,
            |_| Ok(vec![255; 64]),
            |backend, completed, total| {
                assert_eq!(total, 3);
                assert_eq!(backend.textures.borrow().len(), completed);
                seen.push(completed);
                if completed == stop {
                    Err("cancelled by owner")
                } else {
                    Ok(())
                }
            },
        );
        assert!(matches!(
            result,
            Err(SceneError::Backend("cancelled by owner"))
        ));
        assert_eq!(seen, (0..=stop).collect::<Vec<_>>());
        assert!(
            textures.borrow().is_empty(),
            "partial textures at boundary {stop}"
        );
    }
}

#[test]
fn paged_checkpoints_do_not_skip_pages_or_publish_before_lightmap() {
    let atlas = crate::atlas::pack_atlas(
        &[crate::atlas::SpriteSize {
            width: 1,
            height: 1,
        }; 3],
        4,
        1,
        3,
    )
    .unwrap();
    let backend = Backend::default();
    let textures = backend.textures.clone();
    let mut seen = Vec::new();
    let scene = WorldScene::new_paged_checked(
        backend,
        atlas,
        &[255; 1024],
        1,
        |_| Ok(vec![255; 64]),
        |_, completed, total| {
            seen.push((completed, total));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(seen, [(0, 3), (1, 3), (2, 3), (3, 3)]);
    assert_eq!(textures.borrow().len(), 4); // three terrain pages + final lightmap
    drop(scene);
    assert!(textures.borrow().is_empty());
}
