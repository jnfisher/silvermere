//! [`ImportTx`]: every table `silvermere import` writes, in one transaction (issue 34).
//!
//! An import used to write each table in a transaction of its own, so one that failed part
//! way (a character whose first name another account's character already had) left the
//! rooms, shops and monsters of the new world beside the characters of the old, and the
//! `world_import` marker unfinished: the server then refused to start until another import
//! finished. Here the tables and the marker commit together, or not at all: an import that
//! fails or is cut off leaves the database as it was.

use silvermere_types::ids::{AccountId, CharacterId, ShopNo};
use silvermere_types::records::{Action, TextBlock};
use silvermere_types::records::{
    ActiveMonster, BankBook, Character, Class, Gang, Item, ItemOwnership, KnownMonster, Message,
    Race, Room, Shop, Spell,
};
use sqlx::{Any, Transaction};

use crate::Result;
use crate::pool::Db;
use crate::repo::{Account, AccountRow, CharacterRow, StateWrite};
use crate::repo::{
    accounts, active_monsters, bank_books, characters, classes, gangs, item_ownership, items,
    messages, monsters, races, rooms, shops, spells, text_blocks,
};

/// An import's writes, uncommitted until [`ImportTx::commit`]. Dropped without it, every
/// write is rolled back. The methods are the repositories' of the same names, run on the
/// transaction.
pub struct ImportTx<'a> {
    db: &'a Db,
    tx: Transaction<'static, Any>,
    started: String,
}

impl Db {
    /// Begins an import's writes ([`ImportTx`]).
    pub async fn begin_import(&self) -> Result<ImportTx<'_>> {
        Ok(ImportTx {
            db: self,
            tx: self.pool().begin().await?,
            started: jiff::Timestamp::now().to_string(),
        })
    }
}

impl ImportTx<'_> {
    /// [`crate::repo::Rooms::put_many`].
    pub async fn rooms(&mut self, rows: &[Room], state: StateWrite) -> Result<()> {
        rooms::put_many(&mut self.tx, rows, state).await
    }

    /// [`crate::repo::Items::put_many`].
    pub async fn items(&mut self, rows: &[Item], state: StateWrite) -> Result<()> {
        items::put_many_in(&mut self.tx, rows, state).await
    }

    /// [`crate::repo::Monsters::put_many`].
    pub async fn monsters(&mut self, rows: &[KnownMonster], state: StateWrite) -> Result<()> {
        monsters::put_many_in(&mut self.tx, rows, state).await
    }

    /// [`crate::repo::Spells::put_many`].
    pub async fn spells(&mut self, rows: &[Spell]) -> Result<()> {
        spells::put_many_in(&mut self.tx, rows).await
    }

    /// [`crate::repo::Classes::put_many`].
    pub async fn classes(&mut self, rows: &[Class]) -> Result<()> {
        classes::put_many_in(&mut self.tx, rows).await
    }

    /// [`crate::repo::Races::put_many`].
    pub async fn races(&mut self, rows: &[Race]) -> Result<()> {
        races::put_many_in(&mut self.tx, rows).await
    }

    /// [`crate::repo::Shops::put_many`].
    pub async fn shops(&mut self, rows: &[Shop], state: StateWrite) -> Result<()> {
        shops::put_many_in(&mut self.tx, rows, state).await
    }

    /// [`crate::repo::Messages::put_many`].
    pub async fn messages(&mut self, rows: &[Message]) -> Result<()> {
        messages::put_many_in(&mut self.tx, rows).await
    }

    /// [`crate::repo::TextBlocks::put_many`].
    pub async fn text_blocks(&mut self, rows: &[TextBlock]) -> Result<()> {
        text_blocks::put_texts_in(&mut self.tx, rows).await
    }

    /// [`crate::repo::Actions::put_many`].
    pub async fn actions(&mut self, rows: &[Action]) -> Result<()> {
        text_blocks::put_actions(&mut self.tx, rows).await
    }

    /// [`crate::repo::ActiveMonsters::put_many`].
    pub async fn active_monsters(
        &mut self,
        rows: &[ActiveMonster],
        state: StateWrite,
    ) -> Result<()> {
        active_monsters::put_many_in(&mut self.tx, rows, state).await
    }

    /// [`crate::repo::Gangs::put_many`].
    pub async fn gangs(&mut self, rows: &[Gang], state: StateWrite) -> Result<()> {
        gangs::put_many_in(&mut self.tx, self.db, rows, state).await
    }

    /// [`crate::repo::ItemOwnerships::put_many`].
    pub async fn item_ownership(
        &mut self,
        rows: &[ItemOwnership],
        state: StateWrite,
    ) -> Result<()> {
        item_ownership::put_many_in(&mut self.tx, rows, state).await
    }

    /// [`crate::repo::Accounts::by_username`].
    pub async fn account_by_username(&mut self, username: &str) -> Result<Option<AccountRow>> {
        accounts::by_username(&mut self.tx, self.db.dialect(), username).await
    }

    /// [`crate::repo::Accounts::insert`].
    pub async fn insert_account(&mut self, account: &Account) -> Result<AccountId> {
        accounts::insert(&mut self.tx, account).await
    }

    /// [`crate::repo::Accounts::update`].
    pub async fn update_account(&mut self, id: AccountId, account: &Account) -> Result<bool> {
        accounts::update(&mut self.tx, id, account).await
    }

    /// [`crate::repo::Characters::by_account`].
    pub async fn character_by_account(
        &mut self,
        account: AccountId,
    ) -> Result<Option<CharacterRow>> {
        characters::by_account(&mut self.tx, account).await
    }

    /// [`crate::repo::Characters::insert`].
    pub async fn insert_character(
        &mut self,
        account: AccountId,
        c: &Character,
    ) -> Result<CharacterId> {
        characters::insert(&mut self.tx, account, c).await
    }

    /// [`crate::repo::Characters::update`].
    pub async fn update_character(&mut self, id: CharacterId, c: &Character) -> Result<bool> {
        characters::update(&mut self.tx, id, c).await
    }

    /// [`crate::repo::BankBooks::get`].
    pub async fn bank_book(
        &mut self,
        account: AccountId,
        bank: ShopNo,
    ) -> Result<Option<BankBook>> {
        bank_books::get(&mut self.tx, account, bank).await
    }

    /// [`crate::repo::BankBooks::put`].
    pub async fn put_bank_book(&mut self, account: AccountId, book: &BankBook) -> Result<()> {
        bank_books::put(&mut self.tx, account, book).await
    }

    /// Marks the import finished ([`crate::world_import`]) and commits every write.
    pub async fn commit(mut self) -> Result<()> {
        crate::world_import::finished_in(&mut self.tx, &self.started).await?;
        self.tx.commit().await?;
        Ok(())
    }
}
