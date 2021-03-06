use derivative::Derivative;
use serde::{Deserialize, Serialize};
use smallvec::{smallvec, SmallVec};
use std::collections::HashSet;
use std::fmt::Write;
use std::ops::{Add, AddAssign, Mul};
//use rand::{Rng, SeedableRng};
use rand::seq::SliceRandom;

use crate::actions::*;
pub use crate::simulation_state::cards::CardBehavior;
pub use crate::simulation_state::monsters::MonsterBehavior;
use crate::simulation_state::*;

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug, Derivative)]
pub struct Distribution(pub SmallVec<[(f64, i32); 4]>);
impl From<i32> for Distribution {
  fn from(value: i32) -> Distribution {
    Distribution(smallvec![(1.0, value)])
  }
}
impl Mul<f64> for Distribution {
  type Output = Distribution;
  fn mul(mut self, other: f64) -> Distribution {
    for pair in &mut self.0 {
      pair.0 *= other;
    }
    self
  }
}
impl Add<Distribution> for Distribution {
  type Output = Distribution;
  fn add(mut self, other: Distribution) -> Distribution {
    self += other;
    self
  }
}
impl AddAssign<Distribution> for Distribution {
  fn add_assign(&mut self, other: Distribution) {
    for (weight, value) in other.0 {
      if let Some(existing) = self
        .0
        .iter_mut()
        .find(|(_, existing_value)| *existing_value == value)
      {
        existing.0 += weight;
      } else {
        self.0.push((weight, value));
      }
    }
  }
}
impl Distribution {
  pub fn new() -> Distribution {
    Distribution(SmallVec::new())
  }
  pub fn split(
    probability: f64,
    then_value: impl Into<Distribution>,
    else_value: impl Into<Distribution>,
  ) -> Distribution {
    (then_value.into() * probability) + (else_value.into() * (1.0 - probability))
  }
}

/*
pub enum CardChoiceType {
  ExhaustCard,
  HandTopdeck,
  DiscardTopdeck,
  TutorSkill,
  TutorAttack,
}
*/

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Debug)]
pub enum CreatureIndex {
  Player,
  Monster(usize),
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Debug)]
pub enum DamageType {
  Normal,
  Thorns,
  HitpointLoss,
}

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Debug)]
pub struct DamageInfo {
  pub damage_type: DamageType,
  pub owner: CreatureIndex,
  pub base: i32,
  pub output: i32,
}

impl DamageInfo {
  pub fn new(source: CreatureIndex, base: i32, damage_type: DamageType) -> DamageInfo {
    DamageInfo {
      owner: source,
      base,
      damage_type,
      output: base,
    }
  }
  pub fn apply_powers(&mut self, state: &CombatState, owner: CreatureIndex, target: CreatureIndex) {
    self.output = self.base;
    let mut damage = self.output as f64;
    power_hook!(
      state,
      owner,
      damage = at_damage_give(damage, self.damage_type)
    );
    power_hook!(
      state,
      target,
      damage = at_damage_receive(damage, self.damage_type)
    );
    power_hook!(
      state,
      target,
      damage = at_damage_final_receive(damage, self.damage_type)
    );
    self.output = damage as i32;
    if self.output < 0 {
      self.output = 0
    }
  }
}

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Debug)]
pub enum PowerType {
  Buff,
  Debuff,
  Relic,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug, Derivative)]
#[derivative(Default)]
pub enum Determinism {
  Choice,
  Random(Distribution),
  #[derivative(Default)]
  Deterministic,
}

pub trait Action: Clone + Into<DynAction> {
  #[allow(unused)]
  fn determinism(&self, state: &CombatState) -> Determinism {
    Determinism::Deterministic
  }
  #[allow(unused)]
  fn execute(&self, runner: &mut Runner) {
    panic!("an action didn't define the correct apply method for its determinism")
  }
  #[allow(unused)]
  fn execute_random(&self, runner: &mut Runner, random_value: i32) {
    panic!("an action didn't define the correct apply method for its determinism")
  }
}

pub struct Runner<'a> {
  state: &'a mut CombatState,
  allow_random: bool,
  debug: bool,
  log: String,
}

impl<'a> Runner<'a> {
  pub fn new(state: &'a mut CombatState, allow_random: bool, debug: bool) -> Self {
    Runner {
      state,
      allow_random,
      debug,
      log: String::new(),
    }
  }

  pub fn can_apply_impl(&self, action: &impl Action) -> bool {
    match action.determinism(self.state()) {
      Determinism::Deterministic => true,
      Determinism::Random(distribution) => self.allow_random || distribution.0.len() == 1,
      Determinism::Choice => false,
    }
  }
  pub fn can_apply(&self, action: &impl Action) -> bool {
    self.can_apply_impl(action) && !self.state().combat_over()
  }
  pub fn apply_impl(&mut self, action: &impl Action) {
    if self.debug {
      writeln!(
        self.log,
        "Applying {:?} to state {:?}",
        action.clone().into(),
        self.state
      )
      .unwrap();
    }
    match action.determinism(self.state()) {
      Determinism::Deterministic => action.execute(self),
      Determinism::Random(distribution) => {
        let random_value = distribution
          .0
          .choose_weighted(&mut rand::thread_rng(), |(weight, _)| *weight)
          .unwrap()
          .1;
        action.execute_random(self, random_value);
      }
      Determinism::Choice => unreachable!(),
    }
    if self.debug {
      writeln!(
        self.log,
        "Done applying {:?}; state is now {:?}",
        action.clone().into(),
        self.state
      )
      .unwrap();
    }
  }
  pub fn action_now(&mut self, action: &impl Action) {
    if self.state().fresh_subaction_queue.is_empty() && self.can_apply(action) {
      self.apply_impl(action);
    } else {
      self
        .state_mut()
        .fresh_subaction_queue
        .push(action.clone().into());
    }
  }
  pub fn action_top(&mut self, action: impl Action) {
    self.state_mut().actions.push_front(action.into());
  }
  pub fn action_bottom(&mut self, action: impl Action) {
    self.state_mut().actions.push_back(action.into());
  }

  pub fn state(&self) -> &CombatState {
    self.state
  }
  pub fn state_mut(&mut self) -> &mut CombatState {
    self.state
  }
  pub fn debug_log(&self) -> &str {
    &self.log
  }
}

pub fn run_until_unable(runner: &mut Runner) {
  loop {
    if runner.state().combat_over() {
      break;
    }

    while let Some(action) = runner.state_mut().fresh_subaction_queue.pop() {
      runner.state_mut().stale_subaction_stack.push(action)
    }

    if let Some(action) = runner.state_mut().stale_subaction_stack.pop() {
      if runner.can_apply(&action) {
        runner.action_now(&action);
      } else {
        runner.state_mut().stale_subaction_stack.push(action);
        break;
      }
    } else {
      if let Some(action) = runner.state_mut().actions.pop_front() {
        runner.action_now(&action);
      } else {
        break;
      }
    }
  }
}

/*#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Choice {
  PlayCard(SingleCard, usize),
  EndTurn,
}

impl Choice {
  pub fn apply(&self, state: &mut CombatState, runner: &mut Runner) {
    match self {
      Choice::PlayCard(card, target) => state.play_card(runner, card, *target),
      Choice::EndTurn => state.end_turn(runner),
    }
  }
}*/

pub type Choice = DynAction;

impl Creature {
  pub fn has_power(&self, power_id: PowerId) -> bool {
    self.powers.iter().any(|power| power.power_id == power_id)
  }
  pub fn power_amount(&self, power_id: PowerId) -> i32 {
    self
      .powers
      .iter()
      .filter(|power| power.power_id == power_id)
      .map(|power| power.amount)
      .sum()
  }
}

impl CombatState {
  pub fn combat_over(&self) -> bool {
    self.player.creature.hitpoints <= 0 || self.monsters.iter().all(|monster| monster.gone)
  }

  pub fn card_playable(&self, card: &SingleCard) -> bool {
    assert!(X_COST == -1);
    assert!(UNPLAYABLE == -2);
    card.cost >= -1
      && self.player.energy >= card.cost
      && card.card_info.id.playable(self)
      && !(card.card_info.card_type == CardType::Attack
        && self.player.creature.has_power(PowerId::Entangled))
  }

  pub fn legal_choices(&self) -> Vec<Choice> {
    let mut result = Vec::with_capacity(10);
    result.push(EndTurn.into());
    for (index, card) in self.hand.iter().enumerate() {
      if self.hand[..index]
        .iter()
        .all(|earlier_card| earlier_card != card)
        && self.card_playable(card)
      {
        if card.card_info.has_target {
          for (monster_index, monster) in self.monsters.iter().enumerate() {
            if !monster.gone {
              result.push(
                PlayCard {
                  card: card.clone(),
                  target: monster_index,
                }
                .into(),
              );
            }
          }
        } else {
          result.push(
            PlayCard {
              card: card.clone(),
              target: 0,
            }
            .into(),
          );
        }
      }
    }
    result
  }

  pub fn get_creature(&self, index: CreatureIndex) -> &Creature {
    match index {
      CreatureIndex::Player => &self.player.creature,
      CreatureIndex::Monster(index) => &self.monsters[index].creature,
    }
  }

  pub fn get_creature_mut(&mut self, index: CreatureIndex) -> &mut Creature {
    match index {
      CreatureIndex::Player => &mut self.player.creature,
      CreatureIndex::Monster(index) => &mut self.monsters[index].creature,
    }
  }

  pub fn monster_intent(&self, monster_index: usize) -> i32 {
    self.monsters[monster_index].intent()
  }
}

impl Monster {
  pub fn intent(&self) -> i32 {
    *self.move_history.last().unwrap()
  }
  pub fn push_intent(&mut self, intent: i32) {
    /*if self.move_history.len() == 3 {
      self.move_history.remove(0);
    }*/
    self.move_history.push(intent);
  }
}
