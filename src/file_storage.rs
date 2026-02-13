use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use anyhow::{Result, anyhow};
use tracing::{debug, info};

#[derive(Clone)]
pub struct FileStorage {
    pub dir: PathBuf,
    pub total_size_limit: u64,
    pub current_size: Arc<Mutex<u64>>,
}

impl FileStorage {
    pub async fn new(dir: &Path, total_size_limit: u64) -> Result<Self> {
        if !dir.exists() {
            fs::create_dir_all(dir).await?;
        }
        
        // Calculate current size
        let mut size = 0;
        let mut entries = fs::read_dir(dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if let Ok(metadata) = entry.metadata().await {
                size += metadata.len();
            }
        }
        
        info!("File storage initialized at {:?} with current size {} bytes", dir, size);

        Ok(Self {
            dir: dir.to_path_buf(),
            total_size_limit,
            current_size: Arc::new(Mutex::new(size)),
        })
    }

    pub async fn write(&self, id: &str, mut reader: impl tokio::io::AsyncRead + Unpin) -> Result<u64> {
        let file_path = self.dir.join(id);
        if file_path.exists() {
            return Err(anyhow!("File already exists"));
        }

        // Check limit before writing (approximation, strict check would need to wrap writer)
        {
            let current = *self.current_size.lock().await;
            if current >= self.total_size_limit {
                return Err(anyhow!("Storage limit reached"));
            }
        }

        let mut file = fs::File::create(&file_path).await?;
        
        // Copy and count bytes
        let mut written = 0;
        let mut buffer = [0u8; 8192];
        
        loop {
            let n = reader.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            
            // Check global limit
            {
               let mut current = self.current_size.lock().await;
               if *current + (n as u64) > self.total_size_limit {
                   // Clean up
                   drop(current); // Release lock
                   drop(file); // Close file
                   let _ = fs::remove_file(&file_path).await;
                   return Err(anyhow!("Storage limit reached during write"));
               }
               *current += n as u64;
            }
            
            file.write_all(&buffer[0..n]).await?;
            written += n as u64;
        }
        
        file.flush().await?;
        debug!("Written attachment {} ({} bytes)", id, written);
        
        Ok(written)
    }

    #[allow(dead_code)]
    pub async fn remove(&self, id: &str) -> Result<()> {
        let file_path = self.dir.join(id);
        if file_path.exists() {
            let metadata = fs::metadata(&file_path).await?;
            let len = metadata.len();
            fs::remove_file(&file_path).await?;
            
            let mut current = self.current_size.lock().await;
            if *current >= len {
                *current -= len;
            } else {
                *current = 0;
            }
        }
        Ok(())
    }
    
    pub fn get_path(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }
    
    pub fn exists(&self, id: &str) -> bool {
        self.dir.join(id).exists()
    }

    pub async fn get_total_size(&self) -> u64 {
        *self.current_size.lock().await
    }

    pub async fn remove_by_ids(&self, ids: &[String]) -> Result<u64> {
        let mut count = 0;
        for id in ids {
             if let Ok(_) = self.remove(id).await {
                 count += 1;
             }
        }
        Ok(count)
    }
}
