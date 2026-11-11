use super::*;

#[derive(Clone, Debug)]
pub(super) struct EndpointUsage {
    generation: u64,
    boot_id: String,
    revision: u64,
    snapshot: herdr_usage::UsageSnapshot,
    offset: time::UtcOffset,
}

pub(super) fn snapshot(
    endpoint: &ClientShellEndpoint,
) -> Option<(&herdr_usage::UsageSnapshot, time::UtcOffset)> {
    let usage = endpoint.usage.as_ref()?;
    let baseline = endpoint.snapshot.as_deref()?;
    (endpoint.snapshot_generation == Some(usage.generation) && baseline.boot_id == usage.boot_id)
        .then_some((&usage.snapshot, usage.offset))
}

impl ClientShellState {
    pub(crate) fn set_endpoint_usage(
        &mut self,
        endpoint_id: &ClientEndpointId,
        generation: u64,
        projection: crate::protocol::endpoint::EndpointUsageProjection,
    ) -> bool {
        let Some(endpoint) = self
            .endpoints
            .iter_mut()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
        else {
            return false;
        };
        if endpoint
            .snapshot_generation
            .is_some_and(|current| current > generation)
            || endpoint.usage.as_ref().is_some_and(|old| {
                old.generation == generation
                    && old.boot_id == projection.boot_id
                    && old.revision >= projection.revision
            })
        {
            return false;
        }
        endpoint.usage = Some(EndpointUsage {
            generation,
            boot_id: projection.boot_id,
            revision: projection.revision,
            snapshot: projection.usage,
            offset: time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC),
        });
        endpoint_id == &self.active_endpoint_id
    }
}
