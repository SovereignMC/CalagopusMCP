/// Must match `package_name` with dots replaced by underscores, which is what the panel's
/// extension installer derives the crate name and the install directory from.
pub const PACKAGE: &str = "dev_sovereignmc_mcpserver";

pub mod mcp;
pub mod openapi;
pub mod panel;
pub mod tools;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub struct ExtensionStruct;

#[async_trait::async_trait]
impl shared::extensions::Extension for ExtensionStruct {
    async fn initialize_router(
        &mut self,
        state: shared::State,
        builder: shared::extensions::ExtensionRouteBuilder,
    ) -> shared::extensions::ExtensionRouteBuilder {
        builder.add_client_api_router(|routes| {
            routes.nest(&format!("/extensions/{PACKAGE}"), crate::mcp::router(&state))
        })
    }
}
