/// Type-state markers for the builder pattern
///
/// These types are used to track which fields have been set
/// in the builder at compile-time, preventing invalid configurations.

use std::marker::PhantomData;

/// Marker trait for URL state
pub trait UrlState {}

/// URL has not been set
pub struct NoUrl;
impl UrlState for NoUrl {}

/// URL has been set
pub struct HasUrl;
impl UrlState for HasUrl {}

/// Marker trait for Router state
pub trait RouterState {}

/// Router has not been set
pub struct NoRouter;
impl RouterState for NoRouter {}

/// Router has been set
pub struct HasRouter;
impl RouterState for HasRouter {}

/// Phantom marker to prevent direct construction
#[derive(Debug, Clone, Copy)]
pub struct TypeState<U, R> {
    _url: PhantomData<U>,
    _router: PhantomData<R>,
}

impl<U, R> TypeState<U, R> {
    pub(crate) fn new() -> Self {
        Self {
            _url: PhantomData,
            _router: PhantomData,
        }
    }
}

impl<U, R> Default for TypeState<U, R> {
    fn default() -> Self {
        Self::new()
    }
}
