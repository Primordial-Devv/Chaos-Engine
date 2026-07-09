use crate::frame::DrawCommand;

/// File de rendu du moteur : reçoit les soumissions en ordre de scène et
/// rend l'ordre de rendu. Clé actuelle : le **material** — un material
/// implique un pipeline et un bind group, le tri stable regroupe donc les
/// deux états GPU ; l'ordre de soumission est préservé à clé égale. La clé
/// grandira (passe, opaque/transparent, profondeur, tri composite
/// pipeline+material) sans changer ce contrat.
#[derive(Default)]
pub struct RenderQueue {
    commands: Vec<DrawCommand>,
}

impl RenderQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn submit(&mut self, command: DrawCommand) {
        self.commands.push(command);
    }

    pub fn clear(&mut self) {
        self.commands.clear();
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Ordre de rendu de la frame. Le tri est adaptatif : quasi gratuit
    /// quand la file est déjà en ordre (présentations répétées d'une même
    /// frame de simulation).
    pub fn ordered(&mut self) -> &[DrawCommand] {
        self.commands.sort_by_key(|command| command.material.index);
        &self.commands
    }
}

#[cfg(test)]
mod tests {
    use chaos_core::Transform;
    use chaos_core::math::Vec3;

    use crate::material::MaterialHandle;
    use crate::mesh::MeshHandle;

    use super::*;

    fn command(material: u32, x: f32) -> DrawCommand {
        DrawCommand {
            mesh: MeshHandle {
                index: 0,
                generation: 0,
            },
            material: MaterialHandle {
                index: material,
                generation: 0,
            },
            transform: Transform::from_translation(Vec3::new(x, 0.0, 0.0)),
        }
    }

    fn signature(queue: &mut RenderQueue) -> Vec<(u32, f32)> {
        queue
            .ordered()
            .iter()
            .map(|command| (command.material.index, command.transform.translation.x))
            .collect()
    }

    #[test]
    fn empty_queue_yields_nothing() {
        let mut queue = RenderQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
        assert!(queue.ordered().is_empty());
    }

    #[test]
    fn submission_order_is_preserved_within_a_material() {
        let mut queue = RenderQueue::new();
        for step in 0u8..4 {
            queue.submit(command(0, f32::from(step)));
        }
        assert_eq!(
            signature(&mut queue),
            vec![(0, 0.0), (0, 1.0), (0, 2.0), (0, 3.0)]
        );
    }

    #[test]
    fn interleaved_materials_are_grouped_stably() {
        let mut queue = RenderQueue::new();
        queue.submit(command(1, 1.0));
        queue.submit(command(0, 2.0));
        queue.submit(command(1, 3.0));
        queue.submit(command(0, 4.0));
        assert_eq!(
            signature(&mut queue),
            vec![(0, 2.0), (0, 4.0), (1, 1.0), (1, 3.0)]
        );
    }

    #[test]
    fn clear_resets_the_queue() {
        let mut queue = RenderQueue::new();
        queue.submit(command(0, 1.0));
        assert_eq!(queue.len(), 1);
        assert!(!queue.is_empty());
        queue.clear();
        assert!(queue.is_empty());
        assert!(queue.ordered().is_empty());
    }
}
