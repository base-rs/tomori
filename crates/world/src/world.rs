// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The world: roots, the items drawn in them with their instances' bounds, lights, and the views they are culled
//! against.
use glam::{Mat4, Vec3};

use crate::bvh::{Bvh, Leaf};
use crate::light::{self, EnvironmentLight, LIGHTS, LIGHTS_PER_INSTANCE, Light, LightId, Lit};
use crate::{Aabb, Tally, Views};

/// A mesh, as the backend that uploaded it names it: the world never reads its vertices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MeshId(pub u32);

/// A material, as the backend names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MaterialId(pub u32);

/// Coordinates that items are drawn in, carried into the views' by their root's transform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RootId(u32);

/// A slot and its generation, so that something removed is never mistaken for what is added in its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    index: u32,
    generation: u32,
}

/// An item kept in a world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ItemId(Key);

/// Where in a frame's order an item is drawn, as `design/rendering.md` orders them: the opaque bodies, what is cut out,
/// the sky, the translucent surfaces, back to front, and last what is laid over them, as arrival's labels are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    #[default]
    Opaque,
    CutOut,
    Sky,
    Translucent,
    Overlay,
}

/// Whether an item stays where it was placed until its environment changes, and is found through the static
/// instances' hierarchy, or moves, as often as each frame, and is tested on its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mobility {
    Static,
    #[default]
    Dynamic,
}

/// Something drawn: in which root, in which layer, static or dynamic, with which mesh and material if the backend names
/// them, and whether it is culled, as it is unless it opts out, as Bevy's `NoFrustumCulling` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Item {
    pub root: RootId,
    pub layer: Layer,
    pub mobility: Mobility,
    pub mesh: Option<MeshId>,
    pub material: Option<MaterialId>,
    pub culled: bool,
}

impl Item {
    /// An item in `root`'s `layer`, `mobility`, culled, with no mesh or material named.
    pub fn new(root: RootId, layer: Layer, mobility: Mobility) -> Self {
        Self {
            root,
            layer,
            mobility,
            mesh: None,
            material: None,
            culled: true,
        }
    }

    /// Drawn with `mesh`.
    pub fn with_mesh(self, mesh: MeshId) -> Self {
        Self {
            mesh: Some(mesh),
            ..self
        }
    }

    /// Drawn with `material`.
    pub fn with_material(self, material: MaterialId) -> Self {
        Self {
            material: Some(material),
            ..self
        }
    }

    /// Never culled, wherever it is.
    pub fn unculled(self) -> Self {
        Self {
            culled: false,
            ..self
        }
    }
}

/// An item with its instances: where each may be drawn, or none for anywhere, the box about them all, or none if one
/// may be anywhere, which the views may see, and the lights each is lit by, its range of `lit`, as last culled.
#[derive(Debug)]
struct Entry {
    item: Item,
    bounds: Vec<Option<Aabb>>,
    about: Option<Aabb>,
    seen: Vec<bool>,
    lists: Vec<[u32; 2]>,
    lit: Vec<Lit>,
}

/// Coordinates, carried into the views' by `transform`; whether they are culled; the views in them, as last culled;
/// their environment light; the hierarchy of their static instances, and whether it must be built again; and the
/// lights that light their items, as last culled, with the lights' slots ranked to find them.
#[derive(Debug)]
struct Root {
    transform: Mat4,
    active: bool,
    views: Views,
    environment: Option<EnvironmentLight>,
    bvh: Bvh,
    built: bool,
    lit: Vec<Light>,
    ranked: Vec<(f32, u32)>,
}

/// Values kept in slots, each with its generation, and the slots free to use again.
#[derive(Debug)]
struct Slots<T> {
    slots: Vec<(u32, Option<T>)>,
    free: Vec<u32>,
}

impl<T> Default for Slots<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }
}

impl<T> Slots<T> {
    fn insert(&mut self, value: T) -> Key {
        match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                slot.1 = Some(value);
                Key {
                    index,
                    generation: slot.0,
                }
            }
            None => {
                self.slots.push((0, Some(value)));
                Key {
                    index: self.slots.len() as u32 - 1,
                    generation: 0,
                }
            }
        }
    }

    fn get(&self, key: Key) -> Option<&T> {
        self.slots
            .get(key.index as usize)
            .filter(|(generation, _)| *generation == key.generation)
            .and_then(|(_, value)| value.as_ref())
    }

    fn get_mut(&mut self, key: Key) -> Option<&mut T> {
        self.slots
            .get_mut(key.index as usize)
            .filter(|(generation, _)| *generation == key.generation)
            .and_then(|(_, value)| value.as_mut())
    }

    fn remove(&mut self, key: Key) -> Option<T> {
        self.get(key)?;
        Some(self.remove_at(key.index))
    }

    /// Empty the slot at `index`, which holds a value, giving it its next generation.
    fn remove_at(&mut self, index: u32) -> T {
        let slot = &mut self.slots[index as usize];
        let value = slot.1.take().expect("a slot removed holds a value");
        slot.0 = slot.0.wrapping_add(1);
        self.free.push(index);
        value
    }

    fn values(&self) -> impl Iterator<Item = (u32, &T)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index, (_, value))| Some((index as u32, value.as_ref()?)))
    }
}

/// The render world: roots, items with their instances, lights, and this frame's views, kept across frames, and what
/// the views may see of it, as last culled. See the crate's documentation.
#[derive(Debug)]
pub struct World {
    roots: Vec<Root>,
    items: Slots<Entry>,
    lights: Slots<(Light, bool)>,
    views: Views,
    culling: bool,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    /// An empty world, culling against no views until it is given them.
    pub fn new() -> Self {
        Self {
            roots: Vec::new(),
            items: Slots::default(),
            lights: Slots::default(),
            views: Views::default(),
            culling: true,
        }
    }

    /// New coordinates, active, carried into the views' by nothing.
    pub fn add_root(&mut self) -> RootId {
        self.roots.push(Root {
            transform: Mat4::IDENTITY,
            active: true,
            views: Views::default(),
            environment: None,
            bvh: Bvh::default(),
            built: true,
            lit: Vec::with_capacity(LIGHTS),
            ranked: Vec::new(),
        });
        RootId(self.roots.len() as u32 - 1)
    }

    /// Carry `root`'s coordinates into the views' by `transform`, from the next cull.
    pub fn set_transform(&mut self, root: RootId, transform: Mat4) {
        self.roots[root.0 as usize].transform = transform;
    }

    /// Cull `root`'s items and lights, or leave them as they were last culled, `false`, while it is not drawn, as a
    /// scenery other than the current one is not.
    pub fn set_active(&mut self, root: RootId, active: bool) {
        self.roots[root.0 as usize].active = active;
    }

    /// Light `root`'s items by `environment`, or say none.
    pub fn set_environment(&mut self, root: RootId, environment: Option<EnvironmentLight>) {
        self.roots[root.0 as usize].environment = environment;
    }

    /// `root`'s environment light, if it has one.
    pub fn environment(&self, root: RootId) -> Option<&EnvironmentLight> {
        self.roots[root.0 as usize].environment.as_ref()
    }

    /// Remove every item and light of `root`, and its environment light, as when what drew them is dropped.
    pub fn clear(&mut self, root: RootId) {
        for index in 0..self.items.slots.len() as u32 {
            if matches!(&self.items.slots[index as usize].1, Some(entry) if entry.item.root == root)
            {
                self.items.remove_at(index);
            }
        }
        for index in 0..self.lights.slots.len() as u32 {
            if matches!(&self.lights.slots[index as usize].1, Some((light, _)) if light.root == root)
            {
                self.lights.remove_at(index);
            }
        }
        let root = &mut self.roots[root.0 as usize];
        root.environment = None;
        root.built = false;
        root.lit.clear();
    }

    /// Add `item`, with no instances until it is placed.
    pub fn add(&mut self, item: Item) -> ItemId {
        ItemId(self.items.insert(Entry {
            item,
            bounds: Vec::new(),
            about: Some(Aabb::EMPTY),
            seen: Vec::new(),
            lists: Vec::new(),
            lit: Vec::new(),
        }))
    }

    /// Remove `item`, if it is here.
    pub fn remove(&mut self, item: ItemId) -> bool {
        let Some(entry) = self.items.remove(item.0) else {
            return false;
        };
        self.changed(&entry.item);
        true
    }

    /// Whether `item` is here.
    pub fn contains(&self, item: ItemId) -> bool {
        self.items.get(item.0).is_some()
    }

    /// What `item` is, if it is here.
    pub fn item(&self, item: ItemId) -> Option<&Item> {
        self.items.get(item.0).map(|entry| &entry.item)
    }

    /// Place `item`'s instances where `bounds` say each may be drawn, in its root's coordinates, or anywhere for none,
    /// in place of those placed before, each seen, and lit by nothing, until it is culled. Placing as many as before
    /// allocates nothing.
    ///
    /// # Panics
    ///
    /// If `item` is not here.
    pub fn place(&mut self, item: ItemId, bounds: impl IntoIterator<Item = Option<Aabb>>) {
        let entry = self.items.get_mut(item.0).expect("an item placed is here");
        entry.bounds.clear();
        entry.bounds.extend(bounds);
        // A box that is not all numbers may be seen, but would vanish from a union, so the item has none about it.
        entry.about = entry.bounds.iter().try_fold(Aabb::EMPTY, |about, bounds| {
            let bounds = (*bounds)?;
            (bounds.is_empty() || bounds.is_finite()).then(|| about.union(bounds))
        });
        entry.seen.clear();
        entry.seen.resize(entry.bounds.len(), true);
        entry.lists.clear();
        entry.lists.resize(entry.bounds.len(), [0, 0]);
        // Room for as many lights as each may be lit by, so that culling never allocates.
        entry.lit.clear();
        entry.lit.reserve(entry.bounds.len() * LIGHTS_PER_INSTANCE);
        let item = entry.item;
        self.changed(&item);
    }

    /// A static item placed or removed changes its root's hierarchy.
    fn changed(&mut self, item: &Item) {
        if item.mobility == Mobility::Static {
            self.roots[item.root.0 as usize].built = false;
        }
    }

    /// How many instances `item` has, or none if it is not here.
    pub fn instances(&self, item: ItemId) -> usize {
        self.items.get(item.0).map_or(0, |entry| entry.bounds.len())
    }

    /// Add `light`.
    pub fn add_light(&mut self, light: Light) -> LightId {
        LightId(self.lights.insert((light, true)))
    }

    /// `light`, to change it, if it is here.
    pub fn light_mut(&mut self, light: LightId) -> Option<&mut Light> {
        self.lights.get_mut(light.0).map(|(light, _)| light)
    }

    /// Remove `light`, if it is here.
    pub fn remove_light(&mut self, light: LightId) -> bool {
        self.lights.remove(light.0).is_some()
    }

    /// Every light, with whether the views may see anything it reaches, as last culled.
    pub fn lights(&self) -> impl Iterator<Item = (&Light, bool)> {
        self.lights
            .values()
            .map(|(_, (light, seen))| (light, *seen))
    }

    /// Draw from the eyes `clips`, each a view and projection from the views' coordinates, from the next cull.
    ///
    /// # Panics
    ///
    /// If there are more than [`crate::EYES`].
    pub fn set_views(&mut self, clips: &[Mat4]) {
        self.views = Views::new(clips);
    }

    /// Cull what the views cannot see, as by default, or see everything, `false`, to compare, from the next cull.
    pub fn set_culling(&mut self, culling: bool) {
        self.culling = culling;
    }

    /// The views, in the views' own coordinates.
    pub fn views(&self) -> &Views {
        &self.views
    }

    /// The views in `root`'s coordinates, as last culled, which cull nothing when culling is off.
    pub fn root_views(&self, root: RootId) -> &Views {
        &self.roots[root.0 as usize].views
    }

    /// Find what the views may see of every active root: its static instances through their hierarchy, built again if
    /// they changed, and its dynamic items one by one, each item's bounds and then its instances'; which of its lights
    /// reach into the views, of which at most [`LIGHTS`] light its items, those nearest the eyes of more, those at the cut
    /// fading as they near the nearest left out ([`light::FADE`] of their reach); and, by
    /// [`light::assign`], which of those light each instance seen.
    pub fn cull(&mut self) {
        let unculled = Views::unculled(self.views.clips());
        for root in &mut self.roots {
            if root.active {
                root.views = if self.culling {
                    self.views.transformed(root.transform)
                } else {
                    unculled.transformed(root.transform)
                };
            }
        }
        for (index, root) in self.roots.iter_mut().enumerate() {
            if root.built || !root.active {
                continue;
            }
            let id = RootId(index as u32);
            root.bvh.build(
                self.items
                    .slots
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, (_, entry))| {
                        let entry = entry.as_ref()?;
                        (entry.item.root == id
                            && entry.item.mobility == Mobility::Static
                            && entry.item.culled)
                            .then_some((slot, entry))
                    })
                    .flat_map(|(slot, entry)| {
                        entry
                            .bounds
                            .iter()
                            .enumerate()
                            .filter_map(move |(instance, bounds)| {
                                let bounds = (*bounds)?;
                                (bounds.is_finite() && !bounds.is_empty()).then_some(Leaf {
                                    bounds,
                                    slot: slot as u32,
                                    instance: instance as u32,
                                })
                            })
                    }),
            );
            root.built = true;
        }
        // Each item's instances, but static ones in a hierarchy, which it then marks.
        for (_, entry) in self.items.slots.iter_mut() {
            let Some(entry) = entry else { continue };
            let root = &self.roots[entry.item.root.0 as usize];
            if !root.active {
                continue;
            }
            let views = &root.views;
            if !entry.item.culled || views.frustum().is_none() {
                entry.seen.fill(true);
            } else if entry.item.mobility == Mobility::Static {
                // Those the hierarchy does not hold: anywhere, or not all numbers, are seen, as an empty box is not.
                for (seen, bounds) in entry.seen.iter_mut().zip(&entry.bounds) {
                    *seen = bounds.is_none_or(|bounds| !bounds.is_empty() && !bounds.is_finite());
                }
            } else if entry.about.is_some_and(|about| !views.sees(&about)) {
                entry.seen.fill(false);
            } else {
                for (seen, bounds) in entry.seen.iter_mut().zip(&entry.bounds) {
                    *seen = bounds.is_none_or(|bounds| views.sees(&bounds));
                }
            }
        }
        for root in &mut self.roots {
            let Some(frustum) = root.views.frustum().filter(|_| root.active) else {
                continue;
            };
            let items = &mut self.items;
            // The hierarchy is built again whenever a static item of an active root is placed or removed, so each of its
            // leaves is an instance here.
            root.bvh.walk(frustum, |leaf| {
                let entry = items.slots[leaf.slot as usize].1.as_mut();
                entry.expect("a static item in the hierarchy is here").seen
                    [leaf.instance as usize] = true;
            });
        }
        for (_, light) in self.lights.slots.iter_mut() {
            let Some((light, seen)) = light else { continue };
            let root = &self.roots[light.root.0 as usize];
            if root.active {
                let reach = Vec3::splat(light.reach.max(0.0));
                *seen = root
                    .views
                    .sees(&Aabb::new(light.position - reach, light.position + reach));
            }
        }
        self.light();
    }

    /// Choose each active root's lights, then the lights of each instance seen.
    fn light(&mut self) {
        for (index, root) in self.roots.iter_mut().enumerate() {
            if !root.active {
                continue;
            }
            let id = RootId(index as u32);
            // By how far each is from reaching the first eye, less within its reach, then in the order they were added.
            let eye = root.views.eyes().next().unwrap_or(Vec3::ZERO);
            root.ranked.clear();
            root.ranked
                .extend(
                    self.lights
                        .slots
                        .iter()
                        .enumerate()
                        .filter_map(|(slot, (_, light))| {
                            let (light, seen) = light.as_ref()?;
                            (light.root == id && *seen && light.gives_light())
                                .then(|| (light.position.distance(eye) - light.reach, slot as u32))
                        }),
                );
            // Of more, those as near as the nearest left out fade, as they near it, so that none is seen to come or go.
            let mut cut = None;
            if root.ranked.len() > LIGHTS {
                root.ranked
                    .sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                cut = Some(root.ranked[LIGHTS].0);
                root.ranked.truncate(LIGHTS);
                root.ranked.sort_unstable_by_key(|(_, slot)| *slot);
            }
            root.lit.clear();
            root.lit.extend(root.ranked.iter().map(|(gap, slot)| {
                let light = self.lights.slots[*slot as usize]
                    .1
                    .expect("a light ranked is here")
                    .0;
                Light {
                    intensity: light.intensity
                        * cut.map_or(1.0, |cut| light::faded(light.reach, *gap, cut)),
                    ..light
                }
            }));
        }
        for (_, entry) in self.items.slots.iter_mut() {
            let Some(Entry {
                item,
                bounds,
                seen,
                lists,
                lit,
                ..
            }) = entry
            else {
                continue;
            };
            let root = &self.roots[item.root.0 as usize];
            if !root.active {
                continue;
            }
            lit.clear();
            for ((list, seen), bounds) in lists.iter_mut().zip(seen.iter()).zip(bounds.iter()) {
                let start = lit.len();
                if *seen {
                    light::assign(&root.lit, bounds.as_ref(), lit);
                }
                *list = [start as u32, (lit.len() - start) as u32];
            }
        }
    }

    /// The lights that light `root`'s items, as last culled, in the order they were added, which [`Lit::light`] indexes;
    /// of more than [`LIGHTS`], those fading at the cut with their intensity scaled down.
    pub fn root_lights(&self, root: RootId) -> &[Light] {
        &self.roots[root.0 as usize].lit
    }

    /// The lights that light each of `item`'s instances, as last culled, in the order they were placed: none for one
    /// not seen, and nothing for an item not here.
    pub fn lit(&self, item: ItemId) -> impl ExactSizeIterator<Item = &[Lit]> + '_ {
        let (lists, lit) = self.items.get(item.0).map_or((&[][..], &[][..]), |entry| {
            (entry.lists.as_slice(), entry.lit.as_slice())
        });
        lists
            .iter()
            .map(move |[start, count]| &lit[*start as usize..(*start + *count) as usize])
    }

    /// Which of `item`'s instances the views may see, as last culled, in the order they were placed; none for an item
    /// not here.
    pub fn seen(&self, item: ItemId) -> &[bool] {
        self.items
            .get(item.0)
            .map_or(&[], |entry| entry.seen.as_slice())
    }

    /// Whether the views may see any of `item`'s instances, as last culled.
    pub fn visible(&self, item: ItemId) -> bool {
        self.seen(item).iter().any(|seen| *seen)
    }

    /// How many instances of `root`'s items the views may see, as last culled, and how many they may not.
    pub fn tally(&self, root: RootId) -> Tally {
        let (drawn, all) = self
            .items
            .values()
            .filter(|(_, entry)| entry.item.root == root)
            .fold((0, 0), |(drawn, all), (_, entry)| {
                (
                    drawn + entry.seen.iter().filter(|seen| **seen).count(),
                    all + entry.seen.len(),
                )
            });
        Tally {
            drawn,
            culled: all - drawn,
            ..Tally::default()
        }
    }

    /// How many static instances `root`'s hierarchy holds, as last built.
    pub fn static_instances(&self, root: RootId) -> usize {
        self.roots[root.0 as usize].bvh.len()
    }
}

#[cfg(test)]
mod tests;
