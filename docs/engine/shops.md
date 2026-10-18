# Shops

LIST, BUY, SELL and APPRAISE, the restock cycle and the limited-item rule (T-041). Code:
`crates/silvermere-engine/src/commands/shop.rs` (the commands, `buy_item`, `sell_item`),
`src/display/shop.rs` (the LIST screen), `src/shops/mod.rs` (slot kinds, restock queue,
name search, the limited-copy test). Prices are `silvermere_rules::economy::buy_price` and
`sell_price`.

## Commands

| Command | Outside a shop room | In one |
|---|---|---|
| `LIST` | `You cannot LIST if you are not in a shop!` | the goods in stock (`display_shop_items`); a bank shows its rate card; a shop with nothing in stock shows nothing |
| `LIST x` | not a command: the line goes on down the input pipeline | same |
| `BUY` | `Syntax: BUY {item}` | same |
| `BUY x` | `BUY ROOM`: the gang house line; a room with a command text gets the line (T-054 hook); else `You cannot BUY if you are not in a shop!` | `buy_item`, then the room's command text sees the line and its verdict is returned |
| `SELL x` | `You cannot SELL if you are not in a shop!` | `sell_item` |
| `APPRAISE x` | not a command in a room with no shop number (the room type is not checked) | `sell_item` with `appraise` |

Every command but APPRAISE ends resting and meditating first; BUY and SELL in a shop also
end sneaking and hiding. STOCK, UNSTOCK and MARKUP (gang shop owners) are T-044's.

## Buying (`buy_item`)

1. A hospital (type 5) sells `healing` (2 copper per missing hit point) and `curing` /
   `cure poison` (25 silver; 15 when not poisoned, charged anyway; every spell on the player
   with the Poison ability ends). A training hall (type 8) sells `training` (`train_level`).
2. Anything else is searched among the shop's goods (`get_shop_item_from_name`): an exact
   name wins, one candidate is the item, several give the "Please be more specific" list
   (the second candidate first, then the first, then the rest in slot order).
3. The slot: every slot listing the item with stock is looked at and the last one is sold
   from. A gang shop uses only slots 0-9 and its asking price (`regentime`) in its currency
   (`regennum`); a deed shop applies the gang house rules first.
4. The price in copper: `cost * (markup+100)/100 * (110 - cha/5)/100`, each division on its
   own; over 100,000 copper the sum is done in hundreds.
5. The item goes into the pack (`add_item_to_inventory`: weight, slots, the limited-item
   record), the room sees `You see X buy a Y.`, the buyer `You just bought Y for <coins>.`
   and the slot loses one copy. A gang shop slot sold out is cleared (`regennum` 5).

### Reason codes

| Code | Meaning | Message |
|---|---|---|
| 0 | no goods of that name | `x is not a known item.` (the typed name) |
| 1 | out of stock, or a limited item with no free copy (then the slot loses one copy) | `You cannot buy x here!` |
| 2 | the purse is short | `You cannot afford x.` |
| 4 | the buyer already carries a gang house deed (sic: unreachable, he gets 5) | `You are already the owner of a gang house.` |
| 5 | the gang has less than `gang_exp * 10000` experience | `Your gang does not have enough experience ...` |
| 6 | not the leader of a gang | `You must be a gang leader to purchase a gang house deed.` |
| 7 | `PlayerFlags::SOLDDEED`: sold a deed before | `Due to outstanding paper-work ...` |

## Selling and appraising (`sell_item`)

A gang shop buys nothing. A worn cursed item sells only when a second copy is carried. The
offer is `cost * (cha/2 + 25)/100` with the cost in copper, always paid in copper, then the purse is changed
up into the fewest coins. Every slot listing the item below its ceiling (`qty < maxqty`)
takes the copy back on the shelf; a `maxqty` 0 slot (a recycler) never does. A deed shop
marks the seller `PlayerFlags::SOLDDEED` instead, on APPRAISE too (sic).

## Limited items

As in the original, a limited copy (`limit > 0`) on a shop's shelf counts as in play: it is
an `OWN_ROOM` record of the shop's room. Unlike the original, that record is marked as the
shelf (`userid` = `items::limited::SHELF_MARK`), so a copy dropped on the shop's floor (a plain
`OWN_ROOM` record) is never taken for a shelf copy. The original could not tell them apart.

- SELL records the copy on the shelf. When every allowed copy is already in play, every
  slot holding the item gives one copy up instead.
- BUY takes the shelf record when there is one. The buyer's record replaces it, so the count
  nets out unchanged. When the buyer cannot pay, the record goes back. It also goes
  back when the item does not fit in the pack; the original lost it there.
- Stock with no record (the shop data's own stock, and restocks) needs a copy free under the
  limit. Otherwise BUY refuses with reason 1 and the slot loses one copy. In the original,
  buying unrecorded stock always lowered `active`, so that refusal never happened and
  unrecorded stock could take the count past `limit`.
- Restocks do not record copies, as in the live original (its restock makes no
  ledger call). The sysop's database reindex records one per copy on a shelf.

## Restocking

- At start-up, shops 0-199 (`shops::NRESTOCK`) are initiated: a slot with no
  interval (`regentime` 0) is topped up once (by `regennum` when a 1-99 roll, `genrdn(1, 100)`, is below
  `regenpct`, never past `maxqty`) or cut back to `maxqty`; a timed slot is cut back to
  `maxqty` and queued, its first restock due `genrdn(1, max(2, regentime))` minutes away: 1 to `regentime` − 1, or exactly 1
  when `regentime` is under 2.
- Every 21st slow cycle (`scheduler::RESTOCK_SLOW` 20, slow cycle 30 s: every 630 s)
  `restock_items` looks at the first 1000 queued entries from the head (the most recently
  queued first, sic): each one due, below its ceiling, adds `regennum` copies when a 1-99
  roll is below `regenpct` (capped at `maxqty`), and is rescheduled one full `regentime`
  later whether or not it restocked.
- Gang shops are never initiated or restocked. Times are fast ticks (one a second) where
  the original used the wall clock to the minute.
- A change of stock is saved through `PersistRequest::ShopStock` (the `shop_stock` table);
  a content reload keeps the live stock.

## LIST screen

`The following items are for sale here:`, a blank line, the green/cyan header, a rule of
54 dashes, then per slot in stock `%-29.29s %-8d %5d %-8s` (the item's cost plus markup in
its own currency, no Charm) or `%-29.29s %-8d     Free`, ending ` (Too powerful)` or
` (You can't use)` when `user_can_use` refuses. ` (Too powerful)` appears only for a scroll
whose linked spell's level is above the player's (as `user_can_use_spell` judges it); the item's own `level` field is not read.
