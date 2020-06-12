use ckb_logger::configure_logger_filter;
use jsonrpc_core::Result;
use jsonrpc_derive::rpc;
use std::time;

#[rpc(server)]
pub trait DebugRpc {
    #[rpc(name = "jemalloc_profiling_dump")]
    fn jemalloc_profiling_dump(&self) -> Result<String>;
    #[rpc(name = "set_logger_filter")]
    fn set_logger_filter(&self, filter: String) -> Result<()>;
}

pub(crate) struct DebugRpcImpl {}

impl DebugRpc for DebugRpcImpl {
    fn jemalloc_profiling_dump(&self) -> Result<String> {
        let timestamp = time::SystemTime::now()
            .duration_since(time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let filename = format!("ckb-jeprof.{}.heap", timestamp);
        ckb_memory_tracker::jemalloc_profiling_dump(filename.clone());
        Ok(filename)
    }

    fn set_logger_filter(&self, filter: String) -> Result<()> {
        configure_logger_filter(&filter);
        Ok(())
    }
}
