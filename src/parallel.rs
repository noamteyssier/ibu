use crate::{Record, Result};

/// Trait for types that can process records in parallel.
///
/// This is implemented by the **processor** not by the **reader**.
/// For the **reader**, see the [`ParallelReader`] trait.
pub trait ParallelProcessor: Send + Clone {
    /// Process a single record
    fn process_record(&mut self, record: Record) -> Result<()>;

    /// Called when a thread finishes processing its batch
    /// Default implementation does nothing
    #[allow(unused_variables)]
    fn on_batch_complete(&mut self) -> Result<()> {
        Ok(())
    }

    /// Set the thread ID for this processor
    ///
    /// Each thread should call this method with its own unique ID.
    #[allow(unused_variables)]
    fn set_tid(&mut self, _tid: usize) {
        // Default implementation does nothing
    }

    /// Get the thread ID for this processor
    fn get_tid(&self) -> Option<usize> {
        None
    }
}

/// Trait for IBU readers that can process records in parallel
///
/// This is implemented by the **reader** not by the **processor**.
/// For the **processor**, see the [`ParallelProcessor`] trait.
pub trait ParallelReader {
    fn process_parallel<P: ParallelProcessor + Clone + 'static>(
        &self,
        processor: P,
        num_threads: usize,
    ) -> Result<()>;
}
