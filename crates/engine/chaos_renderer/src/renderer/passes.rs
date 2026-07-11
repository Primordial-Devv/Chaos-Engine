//! Les PASSES déclarées et leurs files : le registre (labels,
//! dépendances, ordonnancement validés), les caméras par passe, la
//! soumission des draws et le rapport de frame.

use super::*;

impl Renderer {
    /// Soumet un ordre de dessin à la passe principale pour la frame de
    /// simulation courante.
    pub fn queue_draw(&mut self, command: DrawCommand) {
        self.passes[MAIN_PASS].queue.submit(command);
    }

    /// Soumet un ordre de dessin à une passe déclarée. Une passe
    /// désactivée accepte ses draws (la file est vidée au prochain
    /// `clear_draws`, rien n'est rendu) ; un handle inconnu est une
    /// erreur explicite.
    pub fn queue_draw_to(&mut self, pass: PassHandle, command: DrawCommand) -> ChaosResult<()> {
        let Some(record) = self.passes.get_mut(pass.0 as usize) else {
            return Err(ChaosError::Graphics(String::from(
                "render pass handle is unknown",
            )));
        };
        record.queue.submit(command);
        Ok(())
    }

    /// Vide les files de TOUTES les passes — appelée par le moteur au
    /// début de chaque frame de simulation. Les draws survivent ainsi aux
    /// présentations multiples entre deux updates (rafales de redraw du
    /// resize interactif).
    pub fn clear_draws(&mut self) {
        for record in &mut self.passes {
            record.queue.clear();
        }
        self.frame_lights.clear();
        // Le debug de FRAME suit les draws ; les RETENUES survivent —
        // c'est leur raison d'être (elles expirent par le temps).
        self.debug.frame.clear();
    }

    /// Le nombre de draws soumis pour la frame de simulation courante,
    /// TOUTES passes confondues — la jauge des metrics de santé.
    pub fn draw_count(&self) -> usize {
        self.passes.iter().map(|record| record.queue.len()).sum()
    }

    /// Le handle de la passe principale `chaos.main` (surface, ordre 0) —
    /// créée à la construction, toujours présente. La désactiver est le
    /// mécanisme officiel du rendu tout-hors-écran.
    pub fn main_pass(&self) -> PassHandle {
        PassHandle(MAIN_PASS as u32)
    }

    /// Déclare une passe de rendu. La déclaration est VALIDÉE avant
    /// d'entrer au registre : label non vide, unique et hors du préfixe
    /// réservé `chaos.` ; destination et lectures vivantes ; pas de
    /// boucle de feedback déclarée ; et l'invariant d'ordonnancement —
    /// une passe qui écrit une cible lue par une autre doit s'exécuter
    /// AVANT la lectrice. Un refus nomme la règle et laisse le registre
    /// intact. Les passes sont permanentes en V1 : pas de suppression,
    /// la désactivation (`set_pass_enabled`) en tient lieu.
    pub fn add_pass(&mut self, descriptor: &RenderPassDescriptor) -> ChaosResult<PassHandle> {
        descriptor.validate()?;
        if descriptor.label.starts_with("chaos.") {
            return Err(ChaosError::Graphics(format!(
                "render pass '{}': the 'chaos.' label prefix is reserved for engine passes",
                descriptor.label
            )));
        }
        if self
            .passes
            .iter()
            .any(|record| record.descriptor.label == descriptor.label)
        {
            return Err(ChaosError::Graphics(format!(
                "render pass '{}' already exists: labels are unique",
                descriptor.label
            )));
        }
        self.check_pass_resources(descriptor)?;
        let mut candidates: Vec<&RenderPassDescriptor> = self
            .passes
            .iter()
            .map(|record| &record.descriptor)
            .collect();
        candidates.push(descriptor);
        self.validate_ordering(&candidates)?;
        let handle = PassHandle(self.passes.len() as u32);
        debug!(
            "render pass '{}' declared (order {}, {handle:?})",
            descriptor.label, descriptor.order
        );
        self.passes.push(PassRecord {
            descriptor: descriptor.clone(),
            queue: RenderQueue::new(),
        });
        Ok(handle)
    }

    /// Remplace le descripteur d'une passe déclarée — SA file de draws
    /// est conservée. Mêmes validations qu'à la déclaration, revalidées
    /// sur l'ENSEMBLE du registre (changer l'ordre d'une passe peut
    /// casser l'invariant entre deux autres) ; un refus laisse tout
    /// intact. C'est le chemin du redimensionnement d'une cible : le
    /// resize fait tourner le handle, `update_pass` rebranche la passe
    /// (et la réactive si elle s'était auto-désactivée). La passe
    /// principale est protégée : sa destination (la surface) et son
    /// label ne changent pas — load, caméra, ordre et activation restent
    /// libres.
    pub fn update_pass(
        &mut self,
        pass: PassHandle,
        descriptor: &RenderPassDescriptor,
    ) -> ChaosResult<()> {
        let index = pass.0 as usize;
        if self.passes.get(index).is_none() {
            return Err(ChaosError::Graphics(String::from(
                "render pass handle is unknown",
            )));
        }
        descriptor.validate()?;
        if index == MAIN_PASS {
            if descriptor.destination != RenderDestination::Surface {
                return Err(ChaosError::Graphics(String::from(
                    "the main pass renders to the surface: its destination cannot change",
                )));
            }
            if descriptor.label != self.passes[MAIN_PASS].descriptor.label {
                return Err(ChaosError::Graphics(String::from(
                    "the main pass label cannot change",
                )));
            }
        } else {
            if descriptor.label.starts_with("chaos.") {
                return Err(ChaosError::Graphics(format!(
                    "render pass '{}': the 'chaos.' label prefix is reserved for engine passes",
                    descriptor.label
                )));
            }
            if self.passes.iter().enumerate().any(|(other, record)| {
                other != index && record.descriptor.label == descriptor.label
            }) {
                return Err(ChaosError::Graphics(format!(
                    "render pass '{}' already exists: labels are unique",
                    descriptor.label
                )));
            }
        }
        self.check_pass_resources(descriptor)?;
        let candidates: Vec<&RenderPassDescriptor> = self
            .passes
            .iter()
            .enumerate()
            .map(|(other, record)| {
                if other == index {
                    descriptor
                } else {
                    &record.descriptor
                }
            })
            .collect();
        self.validate_ordering(&candidates)?;
        if index == MAIN_PASS
            && let PassLoad::Clear(color) = descriptor.load
        {
            self.last_clear_color = color;
        }
        self.passes[index].descriptor = descriptor.clone();
        debug!(
            "render pass '{}' updated (order {}, {pass:?})",
            descriptor.label, descriptor.order
        );
        Ok(())
    }

    /// Active ou désactive une passe — une passe désactivée est sautée
    /// proprement à chaque frame (visible au rapport), ses draws
    /// acceptés puis vidés sans être rendus.
    pub fn set_pass_enabled(&mut self, pass: PassHandle, enabled: bool) -> ChaosResult<()> {
        let Some(record) = self.passes.get_mut(pass.0 as usize) else {
            return Err(ChaosError::Graphics(String::from(
                "render pass handle is unknown",
            )));
        };
        record.descriptor.enabled = enabled;
        Ok(())
    }

    /// Remplace la caméra d'une passe (sa matrice vue-projection) — le
    /// réglage par frame des caméras dynamiques (ombres, reflets).
    pub fn set_pass_camera(&mut self, pass: PassHandle, view_projection: Mat4) -> ChaosResult<()> {
        let Some(record) = self.passes.get_mut(pass.0 as usize) else {
            return Err(ChaosError::Graphics(String::from(
                "render pass handle is unknown",
            )));
        };
        record.descriptor.view_projection = view_projection;
        Ok(())
    }

    /// Remplace la position monde de la caméra d'une passe — le
    /// spéculaire PBR de la passe.
    pub fn set_pass_camera_position(
        &mut self,
        pass: PassHandle,
        camera_position: Vec3,
    ) -> ChaosResult<()> {
        let Some(record) = self.passes.get_mut(pass.0 as usize) else {
            return Err(ChaosError::Graphics(String::from(
                "render pass handle is unknown",
            )));
        };
        record.descriptor.camera_position = camera_position;
        Ok(())
    }

    /// Le rapport de la dernière frame orchestrée — passe par passe dans
    /// l'ordre d'exécution. Vide avant la première frame ; reconstruit à
    /// chaque `render_frame` ; `render_to_target` n'y touche pas.
    pub fn frame_report(&self) -> &FrameReport {
        &self.report
    }

    /// Les cibles d'une passe doivent être vivantes à sa déclaration —
    /// destination comme lectures.
    fn check_pass_resources(&self, descriptor: &RenderPassDescriptor) -> ChaosResult<()> {
        if let RenderDestination::Target(target) = descriptor.destination
            && self.lifetime.render_target_info(target).is_none()
        {
            return Err(ChaosError::Graphics(format!(
                "render pass '{}': its destination target is stale or already destroyed",
                descriptor.label
            )));
        }
        for read in &descriptor.reads {
            if self.lifetime.render_target_info(*read).is_none() {
                return Err(ChaosError::Graphics(format!(
                    "render pass '{}': a declared read is stale or already destroyed",
                    descriptor.label
                )));
            }
        }
        Ok(())
    }

    /// L'invariant d'ordonnancement, revalidé sur TOUTES les paires à
    /// chaque mutation du registre : si une passe écrit une cible qu'une
    /// autre lit, l'écrivaine doit précéder la lectrice dans l'ordre
    /// d'exécution (tri stable par ordre puis enregistrement). Une
    /// lecture sans écrivain la même frame reste légale (le contenu
    /// d'une frame précédente). L'invariant est DÉCLARATIF : indifférent
    /// à `enabled`.
    fn validate_ordering(&self, descriptors: &[&RenderPassDescriptor]) -> ChaosResult<()> {
        let mut schedule: Vec<usize> = (0..descriptors.len()).collect();
        schedule.sort_by_key(|&index| descriptors[index].order);
        let mut position = vec![0; descriptors.len()];
        for (rank, &index) in schedule.iter().enumerate() {
            position[index] = rank;
        }
        for (reader_index, reader) in descriptors.iter().enumerate() {
            for read in &reader.reads {
                for (writer_index, writer) in descriptors.iter().enumerate() {
                    if writer_index == reader_index {
                        continue;
                    }
                    if writer.destination == RenderDestination::Target(*read)
                        && position[writer_index] > position[reader_index]
                    {
                        let target = self
                            .lifetime
                            .render_target_info(*read)
                            .map(|info| info.label.clone())
                            .unwrap_or_else(|| String::from("render target"));
                        return Err(ChaosError::Graphics(format!(
                            "render pass '{}' writes '{target}' after pass '{}' reads it: schedule it earlier",
                            writer.label, reader.label
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}
