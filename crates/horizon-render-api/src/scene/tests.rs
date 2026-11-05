use super::*;
use std::collections::HashSet;

#[derive(Default)]
struct Backend {
    next_mesh: u64,
    meshes: HashSet<MeshId>,
    fail_remove: bool,
    origins: Vec<[f64; 3]>,
}
impl RenderBackend for Backend {
    type Error = &'static str;
    fn create_texture(&mut self, _: Image<'_>) -> Result<TextureId, Self::Error> {
        Ok(TextureId(1))
    }
    fn update_texture(
        &mut self,
        _: TextureId,
        _: [u32; 2],
        _: Image<'_>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn remove_texture(&mut self, _: TextureId) -> Result<(), Self::Error> {
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
