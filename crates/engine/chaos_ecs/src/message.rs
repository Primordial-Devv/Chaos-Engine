use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;

use chaos_core::Event;

/// Un message : un FAIT publié par un système, lu par d'autres — la
/// communication interne de l'ECS, sans couplage direct. La famille :
/// Component, Resource, Message — même marqueur opt-in, mêmes contraintes.
/// Le nom : `chaos_core::Event` (fenêtre/input) garde le sien — les
/// événements de l'ECS sont des messages typés en file.
pub trait Message: Send + Sync + 'static {}

impl Message for Event {}

struct MessageQueue<T: Message> {
    messages: Vec<T>,
}

impl<T: Message> Default for MessageQueue<T> {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
        }
    }
}

/// Le pont type-erased des files : balayer toutes les files sans
/// connaître les types — le seul besoin du `clear` de frame.
trait AnyMessageQueue: Send + Sync {
    fn clear(&mut self);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Message> AnyMessageQueue for MessageQueue<T> {
    fn clear(&mut self) {
        self.messages.clear();
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Le registre des messages : une file FIFO par type (clé `TypeId`),
/// auto-créée à la première émission — zéro cérémonie d'enregistrement.
/// L'ordre d'émission est l'ordre de lecture : déterminisme. La politique
/// de durée de vie appartient à la boucle moteur (`clear` une fois par
/// frame) — le mécanisme seul vit ici. Pas de journalisation : chemins
/// chauds.
#[derive(Default)]
pub struct Messages {
    queues: HashMap<TypeId, Box<dyn AnyMessageQueue>>,
}

impl Messages {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publie un message en fin de file (la branche d'un downcast raté est
    /// impossible par construction — la clé `TypeId` garantit le type).
    pub fn send<T: Message>(&mut self, message: T) {
        let queue = self
            .queues
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(MessageQueue::<T>::default()));
        if let Some(queue) = queue.as_any_mut().downcast_mut::<MessageQueue<T>>() {
            queue.messages.push(message);
        }
    }

    /// Lit les messages du type dans l'ordre d'émission — type jamais émis
    /// → vide, jamais une erreur.
    pub fn read<T: Message>(&self) -> impl Iterator<Item = &T> {
        self.queues
            .get(&TypeId::of::<T>())
            .and_then(|queue| queue.as_any().downcast_ref::<MessageQueue<T>>())
            .into_iter()
            .flat_map(|queue| queue.messages.iter())
    }

    /// Consomme les messages du type (propriété transférée, ordre
    /// préservé) — la file reste, vide.
    pub fn drain<T: Message>(&mut self) -> impl Iterator<Item = T> {
        self.queues
            .get_mut(&TypeId::of::<T>())
            .and_then(|queue| queue.as_any_mut().downcast_mut::<MessageQueue<T>>())
            .map(|queue| std::mem::take(&mut queue.messages))
            .unwrap_or_default()
            .into_iter()
    }

    /// Balaye TOUTES les files d'un coup — la primitive du balayage de
    /// frame, appelée par la boucle moteur.
    pub fn clear(&mut self) {
        for queue in self.queues.values_mut() {
            queue.clear();
        }
    }
}

impl fmt::Debug for Messages {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Messages")
            .field("queues", &self.queues.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use chaos_core::event::WindowEvent;

    use crate::world::World;

    use super::*;

    #[derive(Debug, PartialEq)]
    struct Ping(u32);

    impl Message for Ping {}

    #[derive(Debug, PartialEq)]
    struct Note(&'static str);

    impl Message for Note {}

    #[test]
    fn an_unsent_type_reads_empty() {
        let mut messages = Messages::new();
        assert_eq!(messages.read::<Ping>().count(), 0);
        messages.clear();
        assert_eq!(messages.read::<Ping>().count(), 0);
    }

    #[test]
    fn send_auto_creates_the_queue_and_read_keeps_fifo_order() {
        let mut messages = Messages::new();
        messages.send(Ping(1));
        messages.send(Ping(2));
        messages.send(Ping(3));
        let values: Vec<u32> = messages.read::<Ping>().map(|ping| ping.0).collect();
        assert_eq!(values, vec![1, 2, 3]);
    }

    #[test]
    fn message_types_have_independent_queues() {
        let mut messages = Messages::new();
        messages.send(Ping(1));
        messages.send(Note("hello"));
        messages.send(Ping(2));
        assert_eq!(messages.read::<Ping>().count(), 2);
        assert_eq!(messages.read::<Note>().count(), 1);
    }

    #[test]
    fn drain_consumes_in_order_and_leaves_the_queue_empty() {
        let mut messages = Messages::new();
        messages.send(Ping(1));
        messages.send(Ping(2));
        let drained: Vec<Ping> = messages.drain::<Ping>().collect();
        assert_eq!(drained, vec![Ping(1), Ping(2)]);
        assert_eq!(messages.read::<Ping>().count(), 0);
        messages.send(Ping(3));
        assert_eq!(messages.read::<Ping>().count(), 1);
    }

    #[test]
    fn clear_sweeps_every_queue_at_once() {
        let mut messages = Messages::new();
        messages.send(Ping(1));
        messages.send(Note("hello"));
        messages.clear();
        assert_eq!(messages.read::<Ping>().count(), 0);
        assert_eq!(messages.read::<Note>().count(), 0);
    }

    #[test]
    fn a_real_engine_event_roundtrips_as_a_message() {
        let mut messages = Messages::new();
        messages.send(Event::Window(WindowEvent::Resized {
            width: 800,
            height: 600,
        }));
        let read: Vec<&Event> = messages.read::<Event>().collect();
        assert_eq!(
            read,
            vec![&Event::Window(WindowEvent::Resized {
                width: 800,
                height: 600
            })]
        );
    }

    #[test]
    fn the_registry_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Messages>();
    }

    #[test]
    fn messages_flow_through_the_world() {
        let mut world = World::new();
        world.send_message(Ping(7));
        world.send_message(Note("world"));
        assert_eq!(world.messages::<Ping>().count(), 1);
        let drained: Vec<Note> = world.drain_messages::<Note>().collect();
        assert_eq!(drained, vec![Note("world")]);
        world.send_message(Ping(8));
        world.clear_messages();
        assert_eq!(world.messages::<Ping>().count(), 0);
    }
}
