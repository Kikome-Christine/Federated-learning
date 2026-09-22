//! gRPC / protobuf definitions shared by every binary in the workspace.
pub mod fl {
    tonic::include_proto!("fl");
}
