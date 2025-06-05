use std::{any::TypeId, marker::PhantomData, path::PathBuf, sync::atomic::AtomicU64};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

// TODO: should have type aswell
#[derive(Debug)]
pub struct AssetHandle<T: 'static> {
    pub(crate) id: u64,
    pub(crate) ty_id: TypeId,
    pub(crate) path: Option<PathBuf>,
    pub(crate) ty: PhantomData<T>,
}

impl<T: 'static> AssetHandle<T> {
    #![allow(clippy::new_without_default)]
    pub(crate) fn new() -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            ty_id: TypeId::of::<T>(),
            path: None,
            ty: PhantomData,
        }
    }

    #[inline]
    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn clone_typed<G>(&self) -> AssetHandle<G> {
        AssetHandle::<G> {
            id: self.id,
            ty: PhantomData,
            ty_id: TypeId::of::<T>(),
            path: self.path.clone(), // TODO:
        }
    }
}

impl<T: 'static> PartialEq for AssetHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T: 'static> Eq for AssetHandle<T> {}

impl<T: 'static> std::hash::Hash for AssetHandle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: 'static> Clone for AssetHandle<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            ty: PhantomData,
            ty_id: TypeId::of::<T>(),
            path: self.path.clone(), // TODO:
        }
    }
}
