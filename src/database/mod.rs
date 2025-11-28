pub(crate) mod entities;

use amplify::s;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, DatabaseTransaction, DbErr,
    EntityTrait, ModelTrait, QueryFilter, QueryOrder, TransactionTrait,
};

use crate::{
    database::entities::{prelude::*, *},
    error::{APIError, AppError},
    routes::OperationStatus,
};

pub struct AppDatabase {
    connection: DatabaseConnection,
}

impl AppDatabase {
    pub(crate) fn new(connection: DatabaseConnection) -> Self {
        Self { connection }
    }

    pub(crate) async fn begin_transaction(&self) -> Result<DatabaseTransaction, APIError> {
        Ok(self.get_connection().begin().await?)
    }

    pub(crate) fn get_connection(&self) -> &DatabaseConnection {
        &self.connection
    }

    pub(crate) async fn set_config(&self, config: config::ActiveModel) -> Result<i32, AppError> {
        let res = Config::insert(config).exec(self.get_connection()).await?;
        Ok(res.last_insert_id)
    }

    pub(crate) async fn set_cosigner_op_status(
        &self,
        status: &cosigner_op_status::ActiveModel,
        txn: &DatabaseTransaction,
    ) -> Result<i32, APIError> {
        Ok(CosignerOpStatus::insert(status.clone())
            .exec(txn)
            .await?
            .last_insert_id)
    }

    pub(crate) async fn set_cosigners(
        &self,
        cosigners: Vec<cosigner::ActiveModel>,
    ) -> Result<i32, AppError> {
        Ok(cosigner::Entity::insert_many(cosigners)
            .exec(self.get_connection())
            .await?
            .last_insert_id)
    }

    pub(crate) async fn set_next_address_index(
        &self,
        index: next_address_index::ActiveModel,
    ) -> Result<i32, AppError> {
        Ok(NextAddressIndex::insert(index)
            .exec(self.get_connection())
            .await?
            .last_insert_id)
    }

    pub(crate) async fn set_op_file(
        &self,
        file: op_file::ActiveModel,
        txn: &DatabaseTransaction,
    ) -> Result<i32, APIError> {
        Ok(OpFile::insert(file).exec(txn).await?.last_insert_id)
    }

    pub(crate) async fn set_operation(
        &self,
        operation: operation::ActiveModel,
        txn: &DatabaseTransaction,
    ) -> Result<i32, APIError> {
        Ok(Operation::insert(operation).exec(txn).await?.last_insert_id)
    }

    pub(crate) async fn update_cosigner_op_status(
        &self,
        status: cosigner_op_status::ActiveModel,
        txn: Option<&DatabaseTransaction>,
    ) -> Result<(), APIError> {
        if let Some(txn) = txn {
            status.update(txn).await?;
        } else {
            status.update(self.get_connection()).await?;
        }
        Ok(())
    }

    pub(crate) async fn update_next_address_index(
        &self,
        index: next_address_index::ActiveModel,
        txn: &DatabaseTransaction,
    ) -> Result<(), APIError> {
        index.update(txn).await?;
        Ok(())
    }

    pub(crate) async fn update_operation(
        &self,
        operation: operation::ActiveModel,
        txn: &DatabaseTransaction,
    ) -> Result<(), APIError> {
        operation.update(txn).await?;
        Ok(())
    }

    pub(crate) async fn get_config(&self) -> Result<Option<config::Model>, AppError> {
        Ok(Config::find().one(self.get_connection()).await?)
    }

    pub(crate) async fn get_cosigner_by_idx(
        &self,
        idx: i32,
    ) -> Result<Option<cosigner::Model>, APIError> {
        Ok(Cosigner::find_by_id(idx).one(self.get_connection()).await?)
    }

    pub(crate) async fn get_cosigner_op_status_entry(
        &self,
        cosigner_idx: i32,
        operation_idx: i32,
    ) -> Result<Option<cosigner_op_status::Model>, APIError> {
        Ok(CosignerOpStatus::find()
            .filter(cosigner_op_status::Column::CosignerIdx.eq(cosigner_idx))
            .filter(cosigner_op_status::Column::OperationIdx.eq(operation_idx))
            .one(self.get_connection())
            .await?)
    }

    pub(crate) async fn get_cosigner_op_status_with_cosigners_by_operation_idx(
        &self,
        operation_idx: i32,
    ) -> Result<Vec<(cosigner_op_status::Model, cosigner::Model)>, APIError> {
        let status_entries = CosignerOpStatus::find()
            .filter(cosigner_op_status::Column::OperationIdx.eq(operation_idx))
            .all(self.get_connection())
            .await?;
        let mut result = Vec::new();
        for status in status_entries {
            let cosigner = status
                .find_related(Cosigner)
                .one(self.get_connection())
                .await?
                .ok_or(APIError::Unexpected(s!(
                    "CosignerOpStatus entry missing cosigner"
                )))?;
            result.push((status, cosigner));
        }
        Ok(result)
    }

    pub(crate) async fn get_last_cosigner_processed_op_idx(
        &self,
        cosigner_idx: i32,
    ) -> Result<i32, APIError> {
        Ok(CosignerOpStatus::find()
            .filter(cosigner_op_status::Column::CosignerIdx.eq(cosigner_idx))
            .filter(cosigner_op_status::Column::ProcessedAt.is_not_null())
            .order_by_desc(cosigner_op_status::Column::OperationIdx)
            .one(self.get_connection())
            .await?
            .map(|o| o.operation_idx)
            .unwrap_or(0))
    }

    pub(crate) async fn get_last_operation_idx(&self) -> Result<Option<i32>, APIError> {
        Ok(Operation::find()
            .order_by_desc(operation::Column::Idx)
            .one(self.get_connection())
            .await?
            .map(|op| op.idx))
    }

    pub(crate) async fn get_next_address_index(
        &self,
        txn: Option<&DatabaseTransaction>,
    ) -> Result<next_address_index::Model, APIError> {
        Ok(if let Some(txn) = txn {
            NextAddressIndex::find().one(txn).await?
        } else {
            NextAddressIndex::find().one(self.get_connection()).await?
        }
        .expect("has been created at startup time"))
    }

    pub(crate) async fn get_op_file_by_idx(
        &self,
        idx: i32,
    ) -> Result<Option<op_file::Model>, APIError> {
        Ok(OpFile::find_by_id(idx).one(self.get_connection()).await?)
    }

    pub(crate) async fn get_op_files_by_operation_idx(
        &self,
        operation_idx: i32,
    ) -> Result<Vec<op_file::Model>, APIError> {
        Ok(Operation::find_by_id(operation_idx)
            .one(self.get_connection())
            .await?
            .ok_or(APIError::Unexpected(s!("operation not found")))?
            .find_related(OpFile)
            .all(self.get_connection())
            .await?)
    }

    pub(crate) async fn get_operation_by_idx(
        &self,
        idx: i32,
    ) -> Result<Option<operation::Model>, APIError> {
        Ok(Operation::find_by_id(idx)
            .one(self.get_connection())
            .await?)
    }

    pub(crate) async fn iter_cosigners<E>(&self) -> Result<Vec<cosigner::Model>, E>
    where
        E: From<DbErr>,
    {
        Ok(Cosigner::find().all(self.get_connection()).await?)
    }

    pub(crate) async fn iter_cosigner_op_status_by_operation_idx(
        &self,
        operation_idx: i32,
        txn: &DatabaseTransaction,
    ) -> Result<Vec<cosigner_op_status::Model>, APIError> {
        Ok(CosignerOpStatus::find()
            .filter(cosigner_op_status::Column::OperationIdx.eq(operation_idx))
            .all(txn)
            .await?)
    }

    pub(crate) async fn has_pending_operation(&self) -> Result<bool, APIError> {
        Ok(Operation::find()
            .filter(operation::Column::Status.eq(OperationStatus::Pending))
            .one(self.get_connection())
            .await?
            .is_some())
    }

    pub(crate) async fn has_unprocessed_operation(
        &self,
        cosigner_idx: i32,
    ) -> Result<bool, APIError> {
        Ok(CosignerOpStatus::find()
            .filter(cosigner_op_status::Column::CosignerIdx.eq(cosigner_idx))
            .filter(cosigner_op_status::Column::ProcessedAt.is_null())
            .one(self.get_connection())
            .await?
            .is_some())
    }

    pub(crate) async fn set_transfer_status(
        &self,
        batch_transfer_idx: i32,
        cosigner_idx: i32,
        registered_at: i64,
        accepted: bool,
        txn: &DatabaseTransaction,
    ) -> Result<transfer_status::Model, APIError> {
        if let Some(existing) = TransferStatus::find()
            .filter(transfer_status::Column::BatchTransferIdx.eq(batch_transfer_idx))
            .one(txn)
            .await?
        {
            if existing.accepted == accepted {
                return Ok(existing);
            } else {
                return Err(APIError::TransferStatusMismatch);
            }
        }
        let active_model = transfer_status::ActiveModel {
            batch_transfer_idx: ActiveValue::Set(batch_transfer_idx),
            cosigner_idx: ActiveValue::Set(cosigner_idx),
            registered_at: ActiveValue::Set(registered_at),
            accepted: ActiveValue::Set(accepted),
            ..Default::default()
        };
        let idx = TransferStatus::insert(active_model)
            .exec(txn)
            .await?
            .last_insert_id;
        Ok(transfer_status::Model {
            idx,
            batch_transfer_idx,
            cosigner_idx,
            registered_at,
            accepted,
        })
    }

    pub(crate) async fn get_transfer_status(
        &self,
        batch_transfer_idx: i32,
    ) -> Result<Option<transfer_status::Model>, APIError> {
        Ok(TransferStatus::find()
            .filter(transfer_status::Column::BatchTransferIdx.eq(batch_transfer_idx))
            .one(self.get_connection())
            .await?)
    }
}
