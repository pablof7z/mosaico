use super::*;

impl DaemonState {
    pub(crate) fn drive_delivery_scan(
        &self,
        trigger: &str,
        fact: crate::reconcile::DeliveryScanFact,
    ) -> Result<Vec<crate::reconcile::DeliveryEffect>> {
        crate::delivery_seam::drive(&self.delivery, &self.store, trigger, fact)
    }
}
