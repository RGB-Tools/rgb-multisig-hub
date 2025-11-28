use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Config::Table)
                    .if_not_exists()
                    .col(pk_auto(Config::Idx))
                    .col(tiny_unsigned(Config::ThresholdColored))
                    .col(tiny_unsigned(Config::ThresholdVanilla))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(NextAddressIndex::Table)
                    .if_not_exists()
                    .col(pk_auto(NextAddressIndex::Idx))
                    .col(big_unsigned(NextAddressIndex::Internal))
                    .col(big_unsigned(NextAddressIndex::External))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Cosigner::Table)
                    .if_not_exists()
                    .col(pk_auto(Cosigner::Idx))
                    .col(string_uniq(Cosigner::Xpub))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Operation::Table)
                    .if_not_exists()
                    .col(pk_auto(Operation::Idx))
                    .col(tiny_unsigned(Operation::Type))
                    .col(tiny_unsigned(Operation::Status))
                    .col(big_unsigned(Operation::CreatedAt))
                    .col(integer(Operation::InitiatorIdx))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-operation-initiatoridx")
                            .from(Operation::Table, Operation::InitiatorIdx)
                            .to(Cosigner::Table, Cosigner::Idx)
                            .on_delete(ForeignKeyAction::Restrict)
                            .on_update(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx-operation-status")
                    .table(Operation::Table)
                    .col(Operation::Status)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(OpFile::Table)
                    .if_not_exists()
                    .col(pk_auto(OpFile::Idx))
                    .col(string(OpFile::FileId))
                    .col(tiny_unsigned(OpFile::Type))
                    .col(integer(OpFile::OperationIdx))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-opfile-operationidx")
                            .from(OpFile::Table, OpFile::OperationIdx)
                            .to(Operation::Table, Operation::Idx)
                            .on_delete(ForeignKeyAction::Restrict)
                            .on_update(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx-opfile-operationidx")
                    .table(OpFile::Table)
                    .col(OpFile::OperationIdx)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(CosignerOpStatus::Table)
                    .if_not_exists()
                    .col(pk_auto(CosignerOpStatus::Idx))
                    .col(integer(CosignerOpStatus::CosignerIdx))
                    .col(integer(CosignerOpStatus::OperationIdx))
                    .col(boolean_null(CosignerOpStatus::Ack))
                    .col(big_unsigned_null(CosignerOpStatus::RespondedAt))
                    .col(big_unsigned_null(CosignerOpStatus::ProcessedAt))
                    .col(integer_null(CosignerOpStatus::PsbtOpFileIdx))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-cosigneropstatus-psbtopfileidx")
                            .from(CosignerOpStatus::Table, CosignerOpStatus::PsbtOpFileIdx)
                            .to(OpFile::Table, OpFile::Idx)
                            .on_delete(ForeignKeyAction::Restrict)
                            .on_update(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-cosigneropstatus-cosigneridx")
                            .from(CosignerOpStatus::Table, CosignerOpStatus::CosignerIdx)
                            .to(Cosigner::Table, Cosigner::Idx)
                            .on_delete(ForeignKeyAction::Restrict)
                            .on_update(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-cosigneropstatus-operationidx")
                            .from(CosignerOpStatus::Table, CosignerOpStatus::OperationIdx)
                            .to(Operation::Table, Operation::Idx)
                            .on_delete(ForeignKeyAction::Restrict)
                            .on_update(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx-cosigneropstatus-cosigneridx-operationidx")
                    .table(CosignerOpStatus::Table)
                    .col(CosignerOpStatus::CosignerIdx)
                    .col(CosignerOpStatus::OperationIdx)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx-cosigneropstatus-operationidx")
                    .table(CosignerOpStatus::Table)
                    .col(CosignerOpStatus::OperationIdx)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx-cosigneropstatus-cosigneridx-processedat")
                    .table(CosignerOpStatus::Table)
                    .col(CosignerOpStatus::CosignerIdx)
                    .col(CosignerOpStatus::ProcessedAt)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(TransferStatus::Table)
                    .if_not_exists()
                    .col(pk_auto(TransferStatus::Idx))
                    .col(integer_uniq(TransferStatus::BatchTransferIdx))
                    .col(integer(TransferStatus::CosignerIdx))
                    .col(big_integer(TransferStatus::RegisteredAt))
                    .col(boolean(TransferStatus::Accepted))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-transferstatus-cosigneridx")
                            .from(TransferStatus::Table, TransferStatus::CosignerIdx)
                            .to(Cosigner::Table, Cosigner::Idx)
                            .on_delete(ForeignKeyAction::Restrict)
                            .on_update(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(TransferStatus::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(CosignerOpStatus::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(OpFile::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(Operation::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(Cosigner::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(NextAddressIndex::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(Config::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Config {
    Table,
    Idx,
    ThresholdColored,
    ThresholdVanilla,
}

#[derive(DeriveIden)]
enum NextAddressIndex {
    Table,
    Idx,
    Internal,
    External,
}

#[derive(DeriveIden)]
enum Cosigner {
    Table,
    Idx,
    Xpub,
}

#[derive(DeriveIden)]
enum OpFile {
    Table,
    Idx,
    FileId,
    Type,
    OperationIdx,
}

#[derive(DeriveIden)]
enum Operation {
    Table,
    Idx,
    Type,
    Status,
    CreatedAt,
    InitiatorIdx,
}

#[derive(DeriveIden)]
enum CosignerOpStatus {
    Table,
    Idx,
    CosignerIdx,
    OperationIdx,
    Ack,
    RespondedAt,
    ProcessedAt,
    PsbtOpFileIdx,
}

#[derive(DeriveIden)]
enum TransferStatus {
    Table,
    Idx,
    BatchTransferIdx,
    CosignerIdx,
    RegisteredAt,
    Accepted,
}
