//! Les MESHES : la géométrie matérialisée en buffers GPU possédés —
//! les trois créations, les bounds locaux calculés à la source (la
//! matière du culling), l'inspection, la destruction qui emporte ses
//! organes.

use super::*;

impl Renderer {
    /// Crée un mesh à sommets colorés : téléverse la géométrie (vertex +
    /// index buffers) et l'enregistre comme ressource de rendu. Le mesh
    /// possède ses buffers.
    pub fn create_mesh(&mut self, label: &str, geometry: &Geometry) -> ChaosResult<MeshHandle> {
        let index_bytes = geometry.is_indexed().then(|| geometry.index_bytes());
        let bounds = Self::geometry_bounds(
            label,
            geometry.vertices.iter().map(|vertex| vertex.position),
        );
        self.register_mesh(
            label,
            geometry.vertex_bytes(),
            index_bytes,
            geometry.element_count(),
            ColorVertex::layout(),
            bounds,
        )
    }

    /// Crée un mesh à sommets texturés (position + UV) — même cycle de vie
    /// que `create_mesh`, layout `TexturedVertex`.
    pub fn create_textured_mesh(
        &mut self,
        label: &str,
        geometry: &TexturedGeometry,
    ) -> ChaosResult<MeshHandle> {
        let index_bytes = geometry.is_indexed().then(|| geometry.index_bytes());
        let bounds = Self::geometry_bounds(
            label,
            geometry.vertices.iter().map(|vertex| vertex.position),
        );
        self.register_mesh(
            label,
            geometry.vertex_bytes(),
            index_bytes,
            geometry.element_count(),
            TexturedVertex::layout(),
            bounds,
        )
    }

    /// Crée un mesh ÉCLAIRABLE (position + normale + UV) : téléverse la
    /// géométrie et l'enregistre comme ressource de rendu — le mesh des
    /// materials `Lit`.
    pub fn create_lit_mesh(
        &mut self,
        label: &str,
        geometry: &LitGeometry,
    ) -> ChaosResult<MeshHandle> {
        let index_bytes = geometry.is_indexed().then(|| geometry.index_bytes());
        let bounds = Self::geometry_bounds(
            label,
            geometry.vertices.iter().map(|vertex| vertex.position),
        );
        self.register_mesh(
            label,
            geometry.vertex_bytes(),
            index_bytes,
            geometry.element_count(),
            LitVertex::layout(),
            bounds,
        )
    }

    /// Les BOUNDS locaux d'une géométrie : l'AABB de ses positions —
    /// `None` (géométrie vide ou position non finie, avec warn) = le
    /// mesh ne sera JAMAIS cullé, le défaut sûr.
    fn geometry_bounds(label: &str, positions: impl IntoIterator<Item = [f32; 3]>) -> Option<Aabb> {
        let mut empty = true;
        let bounds = Aabb::from_points(positions.into_iter().map(|position| {
            empty = false;
            Vec3::from(position)
        }));
        if bounds.is_none() && !empty {
            warn!("mesh '{label}' carries non-finite positions — it will never be culled");
        }
        bounds
    }

    fn register_mesh(
        &mut self,
        label: &str,
        vertex_bytes: Vec<u8>,
        index_bytes: Option<Vec<u8>>,
        element_count: u32,
        vertex_layout: VertexLayout,
        bounds: Option<Aabb>,
    ) -> ChaosResult<MeshHandle> {
        let vertex_descriptor = BufferDescriptor::vertex(label, vertex_bytes);
        // La borne device couvre AUSSI le chemin des meshes — le même
        // refus nommé que les buffers publics.
        self.check_buffer_limit(&vertex_descriptor.label, vertex_descriptor.contents.len())?;
        let vertex_buffer = self.backend.create_buffer(&vertex_descriptor)?;
        self.lifetime.register_buffer(
            vertex_buffer,
            &vertex_descriptor.label,
            vertex_descriptor.contents.len() as u64,
            Some(label),
        );
        let index_buffer = match index_bytes {
            Some(bytes) => {
                let index_descriptor = BufferDescriptor::index(format!("{label}.indices"), bytes);
                self.check_buffer_limit(&index_descriptor.label, index_descriptor.contents.len())?;
                let handle = self.backend.create_buffer(&index_descriptor)?;
                self.lifetime.register_buffer(
                    handle,
                    &index_descriptor.label,
                    index_descriptor.contents.len() as u64,
                    Some(label),
                );
                Some(handle)
            }
            None => None,
        };
        let record = MeshRecord {
            vertex_buffer,
            index_buffer,
            element_count,
            vertex_layout,
            bounds,
        };
        let stride = record.vertex_layout.stride;
        let pool_handle = self
            .meshes
            .insert(record)
            .ok_or_else(|| ChaosError::Graphics(String::from("mesh pool capacity exceeded")))?;
        let handle = MeshHandle {
            index: pool_handle.index,
            generation: pool_handle.generation,
        };
        debug!("mesh '{label}' created ({element_count} elements, stride {stride}, {handle:?})");
        Ok(handle)
    }

    /// Les BOUNDS locaux d'un mesh vivant — `None` = le mesh n'en porte
    /// pas (géométrie vide ou dégénérée) et n'est jamais cullé.
    /// L'inspection du culling et du futur éditeur.
    pub fn mesh_bounds(&self, handle: MeshHandle) -> ChaosResult<Option<Aabb>> {
        self.meshes
            .get(PoolHandle {
                index: handle.index,
                generation: handle.generation,
            })
            .map(|record| record.bounds)
            .ok_or_else(|| {
                ChaosError::Graphics(String::from("mesh handle is stale or already destroyed"))
            })
    }

    /// Détruit un mesh : le propriétaire emporte ses buffers — ils partent
    /// en retraite (libération backend différée). Un handle périmé est une
    /// erreur explicite.
    pub fn destroy_mesh(&mut self, handle: MeshHandle) -> ChaosResult<()> {
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        let Some(record) = self.meshes.remove(pool_handle) else {
            return Err(ChaosError::Graphics(String::from(
                "mesh handle is stale or already destroyed",
            )));
        };
        self.lifetime.retire_owned_buffer(record.vertex_buffer);
        if let Some(index_buffer) = record.index_buffer {
            self.lifetime.retire_owned_buffer(index_buffer);
        }
        debug!("mesh released ({handle:?})");
        Ok(())
    }
}
