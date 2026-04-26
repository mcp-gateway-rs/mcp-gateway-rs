use tokio::{
    io,
    runtime::{Builder, Handle, LocalOptions, LocalRuntime, Runtime},
};
use tracing::info;

#[derive(Debug)]
pub struct RuntimeBuilder<'a> {
    single_threaded: bool,
    number_of_threads: usize,
    global_queue_interval: Option<u32>,
    event_interval: Option<u32>,
    max_io_events_per_tick: Option<usize>,
    thread_name: &'a str,
}

pub enum RuntimeType {
    SingleThreaded(Vec<io::Result<Runtime>>),
    MultiThreaded(io::Result<Runtime>),
}

impl Default for RuntimeBuilder<'_> {
    fn default() -> Self {
        Self {
            single_threaded: true,
            number_of_threads: num_cpus::get(),
            global_queue_interval: Default::default(),
            event_interval: Default::default(),
            max_io_events_per_tick: Default::default(),
            thread_name: "mcp-gateway-rs-runtime",
        }
    }
}

impl<'a> RuntimeBuilder<'a> {
    pub fn build(self) -> RuntimeType {
        if self.single_threaded {
            RuntimeType::SingleThreaded(
                (0..self.number_of_threads)
                    .into_iter()
                    .enumerate()
                    .map(|(i, j)| {
                        let mut builder = Builder::new_current_thread();
                        let builder = builder.enable_all().name(self.thread_name);
                        let builder = if let Some(global_queue_interval) = self.global_queue_interval {
                            builder.global_queue_interval(global_queue_interval)
                        } else {
                            builder
                        };

                        let builder = if let Some(event_interval) = self.event_interval {
                            builder.event_interval(event_interval)
                        } else {
                            builder
                        };

                        let builder = if let Some(max_io_events_per_tick) = self.max_io_events_per_tick {
                            builder.max_io_events_per_tick(max_io_events_per_tick)
                        } else {
                            builder
                        };
                        builder.build()
                    })
                    .collect::<Vec<_>>(),
            )
        } else {
            let mut builder = Builder::new_multi_thread();
            let builder = builder.enable_all().name(self.thread_name).worker_threads(self.number_of_threads);
            let builder = if let Some(global_queue_interval) = self.global_queue_interval {
                builder.global_queue_interval(global_queue_interval)
            } else {
                builder
            };

            let builder = if let Some(event_interval) = self.event_interval {
                builder.event_interval(event_interval)
            } else {
                builder
            };

            let builder = if let Some(max_io_events_per_tick) = self.max_io_events_per_tick {
                builder.max_io_events_per_tick(max_io_events_per_tick)
            } else {
                builder
            };
            RuntimeType::MultiThreaded(builder.build())
        }
    }
}
