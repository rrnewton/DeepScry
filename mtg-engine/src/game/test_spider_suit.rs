#[cfg(test)]
mod tests {
    use crate::core::{Card, CardName, CardType, ManaCost, Subtype};
    use crate::game::{mana_engine::ManaEngine, GameState};
    use smallvec::SmallVec;

    #[test]
    fn test_spider_suit_mana_payment() {
        // Create a game with Spider-Suit in hand
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create Spider-Suit (costs {1})
        let spider_suit_id = game.cards.next_id();
        let mut spider_suit = Card::new(spider_suit_id, CardName::from("Spider-Suit"), p1_id);
        spider_suit.mana_cost = ManaCost::from_string("1");
        spider_suit.types = SmallVec::from_vec(vec![CardType::Artifact]);
        spider_suit.subtypes = SmallVec::from_vec(vec![Subtype::from("Equipment")]);
        game.cards.insert(spider_suit_id, spider_suit);

        // Create a Mountain
        let mountain_id = game.cards.next_id();
        let mut mountain = Card::new(mountain_id, CardName::from("Mountain"), p1_id);
        mountain.types = SmallVec::from_vec(vec![CardType::Land]);
        mountain.subtypes = SmallVec::from_vec(vec![Subtype::from("Basic"), Subtype::from("Mountain")]);
        game.cards.insert(mountain_id, mountain);

        // Put Spider-Suit in hand and Mountain on battlefield
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(spider_suit_id);
        }
        game.battlefield.add(mountain_id);

        // Update mana engine
        let mut engine = ManaEngine::new();
        engine.update(&game, p1_id);

        // Check if we can pay for Spider-Suit
        let cost = game.cards.get(spider_suit_id).unwrap().mana_cost;
        println!("Spider-Suit cost: {:?}", cost);
        println!("Can pay: {}", engine.can_pay(&cost));
        println!("Mana sources: {:?}", engine.all_sources());

        assert!(
            engine.can_pay(&cost),
            "Should be able to pay for Spider-Suit with a Mountain"
        );
    }
}
