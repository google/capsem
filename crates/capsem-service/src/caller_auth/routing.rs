//! Mutation constructors admit only handlers whose first extractor is sealed
//! caller proof. A caller DTO, JSON body, or optional actor cannot satisfy it.
use std::sync::Arc;

use axum::{handler::Handler, routing::MethodRouter};

use super::AuthenticatedActor;
use crate::ServiceState;

/// Axum's first tuple member is its extraction mode, followed by arguments.
pub(crate) trait ActorArguments {}
impl<M> ActorArguments for (M, AuthenticatedActor) {}
macro_rules! actor_arguments {
    ($($argument:ident),+) => {
        impl<M, $($argument),+> ActorArguments for (M, AuthenticatedActor, $($argument),+) {}
    };
}
actor_arguments!(A);
actor_arguments!(A, B);
actor_arguments!(A, B, C);
actor_arguments!(A, B, C, D);
actor_arguments!(A, B, C, D, E);
actor_arguments!(A, B, C, D, E, F);
actor_arguments!(A, B, C, D, E, F, G);

macro_rules! mutation {
    ($($method:ident),+) => {$(
        pub(crate) fn $method<H, T>(handler: H) -> MethodRouter<Arc<ServiceState>>
        where
            H: Handler<T, Arc<ServiceState>>,
            T: ActorArguments + 'static,
        {
            axum::routing::$method(handler)
        }
    )+};
}
mutation!(post, put, patch, delete);
