use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            let current = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(current, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn sixteen_mib_text_stays_within_bounded_memory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sixteen-mib.txt");
    let mut file = std::fs::File::create(&path).unwrap();
    let buffer = vec![b'x'; 64 * 1024];
    for _ in 0..256 {
        file.write_all(&buffer).unwrap();
    }
    drop(file);
    drop(buffer);

    PEAK.store(CURRENT.load(Ordering::Relaxed), Ordering::Relaxed);
    let ebook = openitgo_parser::parse_ebook(&path).unwrap();
    assert!(ebook.text.is_some());
    assert!(ebook.chapters.len() > 8);
    let html = openitgo_parser::html::render_chapter_html(&ebook, 3).unwrap();
    assert!(html.len() <= 4 * 1024 * 1024);
    assert!(ebook.chapters.iter().all(|chapter| {
        let text = chapter.text.as_ref().unwrap();
        text.byte_end - text.byte_start <= 512 * 1024
    }));

    let peak = PEAK.load(Ordering::Relaxed);
    eprintln!("16MiB TXT 实时分配峰值: {peak} 字节");
    assert!(
        peak <= 8 * 1024 * 1024,
        "16MiB 输入的实时分配峰值不应超过 8MiB，实际为 {peak} 字节"
    );
}
