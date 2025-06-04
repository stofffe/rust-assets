use crate::handle::AssetHandle;
use std::any;
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
use std::{
    any::Any,
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::Duration,
};

pub type DynAsset = Box<dyn Asset>;
pub type DynRenderAsset = ArcHandle<dyn Any + Send + Sync>;

pub trait Asset: Any + Send + Sync {
    fn load(path: &Path) -> Self
    where
        Self: Sized;

    fn reload(&mut self, _path: &Path) {
        println!("[WARN]: trying to deserialize asset without implementing trait")
    }
    fn write(&mut self, _path: &Path) {
        println!("[WARN]: trying to serialize asset without implementing trait")
    }
}

pub trait RenderAsset: Any {} // might be able to remove and enforce with convert function
pub trait ConvertableRenderAsset: RenderAsset + Send + Sync {
    type SourceAsset: Asset;
    type Params;

    fn convert(source: &Self::SourceAsset, params: &Self::Params) -> Self;
}

impl dyn Asset {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
// impl Arc<dyn RenderAsset> {}

impl dyn RenderAsset {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

pub struct Assets {
    cache: HashMap<AssetHandle<DynAsset>, Option<DynAsset>>,
    render_cache: HashMap<AssetHandle<DynAsset>, DynRenderAsset>,

    load_handles: HashMap<AssetHandle<DynAsset>, PathBuf>,
    load_dirty: HashSet<AssetHandle<DynAsset>>,
    // async loading
    load_sender: mpsc::Sender<(AssetHandle<DynAsset>, DynAsset)>,
    load_receiver: mpsc::Receiver<(AssetHandle<DynAsset>, DynAsset)>,

    // reloading
    reload_handles: HashMap<PathBuf, AssetHandle<DynAsset>>,
    reload_watcher: notify_debouncer_mini::Debouncer<notify_debouncer_mini::notify::FsEventWatcher>,
    reload_receiver: mpsc::Receiver<PathBuf>,
    reload_sender: mpsc::Sender<PathBuf>,
}

impl Assets {
    pub fn new() -> Self {
        let (reload_sender, reload_receiver) = mpsc::channel();
        let (loaded_sender, loaded_receiver) = mpsc::channel();
        let sender_copy = reload_sender.clone();

        let reload_watcher = notify_debouncer_mini::new_debouncer(
            Duration::from_millis(100),
            move |res: notify_debouncer_mini::DebounceEventResult| match res {
                Ok(events) => {
                    for event in events {
                        sender_copy
                            .clone()
                            .send(event.path)
                            .expect("could not send");
                    }
                }
                Err(err) => println!("debounced result error: {}", err),
            },
        )
        .expect("could not create watcher");

        Self {
            cache: HashMap::new(),
            render_cache: HashMap::new(),
            load_dirty: HashSet::new(),
            reload_handles: HashMap::new(),
            load_handles: HashMap::new(),

            reload_receiver,
            reload_sender,
            reload_watcher,

            load_sender: loaded_sender,
            load_receiver: loaded_receiver,
        }
    }

    //
    // Assets
    //

    pub fn insert<T: Asset + 'static>(&mut self, data: T) -> AssetHandle<T> {
        let handle = AssetHandle::<T>::new();
        self.cache.insert(
            handle.clone().clone_typed::<DynAsset>(),
            Some(Box::new(data)),
        );
        handle
    }

    // TODO: add get_or_default (e.g. 1x1 white pixel for image)
    //
    // could return error union [Ok, Invalid, Loading]
    pub fn get<T: Asset + 'static>(&mut self, handle: AssetHandle<T>) -> Option<&T> {
        self.cache
            .get(&handle.clone_typed::<DynAsset>())
            .expect("invalid handle")
            .as_ref()
            .and_then(|asset| asset.as_any().downcast_ref::<T>())
    }

    pub fn get_mut<T: Asset + 'static>(&mut self, handle: AssetHandle<T>) -> Option<&mut T> {
        // invalidate gpu cache
        self.render_cache
            .remove(&handle.clone().clone_typed::<DynAsset>());

        // set dirty
        self.load_dirty
            .insert(handle.clone().clone_typed::<DynAsset>());

        // get value and convert to T
        self.cache
            .get_mut(&handle.clone_typed::<DynAsset>())
            .expect("invalid handle")
            .as_mut()
            .and_then(|asset| asset.as_any_mut().downcast_mut::<T>())
    }

    //
    // Reloading
    //

    pub fn load_sync<T: Asset + 'static>(
        &mut self,
        path: &Path,
        watch: bool,
        write: bool,
    ) -> AssetHandle<T> {
        let path = fs::canonicalize(path).unwrap();

        let data = T::load(&path);
        let handle = AssetHandle::<T>::new();
        self.cache.insert(
            handle.clone().clone_typed::<DynAsset>(),
            Some(Box::new(data)),
        );

        if watch {
            self.reload_watcher
                .watcher()
                .watch(
                    &path,
                    notify_debouncer_mini::notify::RecursiveMode::Recursive,
                )
                .unwrap();

            self.reload_handles
                .insert(path.clone(), handle.clone().clone_typed::<DynAsset>());
        }

        if write {
            self.load_handles
                .insert(handle.clone().clone_typed::<DynAsset>(), path.clone());
        }

        handle
    }

    pub fn load_async<T: Asset + 'static>(
        &mut self,
        path: &Path,
        watch: bool,
        write: bool,
    ) -> AssetHandle<T> {
        let path = fs::canonicalize(path).unwrap();

        let handle = AssetHandle::<T>::new();
        self.cache
            .insert(handle.clone().clone_typed::<DynAsset>(), None);

        let path_clone = path.clone();
        let handle_clone = handle.clone();
        let loaded_sender_clone = self.load_sender.clone();

        std::thread::spawn(move || {
            println!("start async load");
            std::thread::sleep(Duration::from_millis(2000));
            let data = T::load(&path_clone);
            loaded_sender_clone
                .send((handle_clone.clone_typed::<DynAsset>(), Box::new(data)))
                .expect("could not send");
            println!("end async load");
        });

        if watch {
            self.reload_watcher
                .watcher()
                .watch(
                    &path,
                    notify_debouncer_mini::notify::RecursiveMode::Recursive,
                )
                .unwrap();

            self.reload_handles
                .insert(path.clone(), handle.clone().clone_typed::<DynAsset>());
        }

        if write {
            self.load_handles
                .insert(handle.clone().clone_typed::<DynAsset>(), path.clone());
        }

        handle
    }

    // check if any files completed loading and update cache and invalidate render cache
    pub fn poll_loaded(&mut self) {
        for (handle, asset) in self.load_receiver.try_iter() {
            self.cache.insert(handle.clone(), Some(asset));
            self.render_cache.remove(&handle);
        }
    }

    pub fn poll_write(&mut self) {
        for handle in self.load_dirty.drain() {
            if let Some(path) = self.load_handles.get(&handle) {
                let asset = self.cache.get_mut(&handle).expect("invalid handle");
                if let Some(asset) = asset {
                    asset.write(path);
                    println!("write {:?}", path);
                }
            }
        }
    }

    // checks if any files changed and spawns a thread which reloads the data
    pub fn poll_reload(&mut self) {
        for path in self.reload_receiver.try_iter() {
            let handle = self.reload_handles.get_mut(&path).unwrap();

            let asset = self.cache.get_mut(handle).expect("invalid handle");
            if let Some(asset) = asset {
                asset.reload(&path);
                println!("load {:?}", path);
                self.render_cache.remove(handle);
            }
        }
    }

    pub fn force_reload(&self, path: PathBuf) {
        self.reload_sender.send(path).expect("could not send path");
    }

    //
    // Render assets
    //

    pub fn convert<G: ConvertableRenderAsset>(
        &mut self,
        handle: AssetHandle<G::SourceAsset>,
        params: &G::Params,
    ) -> ArcHandle<G> {
        // create new if not in cache
        if !self
            .render_cache
            .contains_key(&handle.clone().clone_typed::<DynAsset>())
        {
            let asset = self.get(handle.clone()).expect("invalid handle"); // TODO: handle
            let converted = G::convert(asset, params);
            self.render_cache.insert(
                handle.clone().clone_typed::<DynAsset>(),
                ArcHandle::new(converted).upcast(),
            );
        }

        // get value and convert to G
        let any_handle = self
            .render_cache
            .get(&handle.clone_typed::<DynAsset>())
            .unwrap();

        any_handle.downcast::<G>()
    }
}

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct ArcHandle<T: ?Sized + 'static> {
    pub handle: Arc<T>,
    id: u64,
}

impl<T: 'static> ArcHandle<T> {
    pub fn new(handle: T) -> Self {
        ArcHandle {
            handle: Arc::new(handle),
            id: NEXT_ID.fetch_add(1, SeqCst),
        }
    }

    #[inline]
    pub fn id(&self) -> u64 {
        self.id
    }
}

impl<T: 'static> Clone for ArcHandle<T> {
    fn clone(&self) -> Self {
        ArcHandle {
            handle: Arc::clone(&self.handle),
            id: self.id,
        }
    }
}

impl<T: 'static> PartialEq for ArcHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T: 'static> Eq for ArcHandle<T> {}

impl<T: 'static> std::hash::Hash for ArcHandle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: 'static> std::ops::Deref for ArcHandle<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.handle.as_ref()
    }
}

impl<T: 'static> AsRef<T> for ArcHandle<T> {
    fn as_ref(&self) -> &T {
        self.handle.as_ref()
    }
}

// any stuff

impl<T: Any + Send + Sync + 'static> ArcHandle<T> {
    pub fn upcast(self) -> ArcHandle<dyn Any + Send + Sync> {
        ArcHandle {
            handle: self.handle as Arc<dyn Any + Send + Sync>,
            id: self.id,
        }
    }
}
impl ArcHandle<dyn Any + Sync + Send> {
    fn downcast<G: Send + Sync>(&self) -> ArcHandle<G> {
        ArcHandle {
            handle: self.handle.clone().downcast::<G>().unwrap(),
            id: self.id,
        }
    }
}
