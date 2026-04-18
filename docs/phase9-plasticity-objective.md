# Phase 9 reopen — `PlasticityObjective` API 설계

**Status**: 설계 초안 (2026-04-18)
**Context**: `docs/complexity-audit.md` §Phase 9 reopen condition (a) — MET.
`docs/sociopatterns-finding.md` §3 — precision@K 가 supervised signal 로 성립.
`docs/phase9-benchmarks.md` — larger-profile suitability / signal-mismatch 확인.

## 0. 리오픈 조건과 설계 제약

`complexity-audit.md` 264-281 이 못박은 조건:

> `PlasticityObjective::*` API 가 **domain declaration 으로 수용**되어야 하며
> **tuning knob 이 아니어야** 한다.

Phase 9 scouting (2026-04-18 오전) 가 롤백된 이유는 "knob 이 `learning_rate`
에서 `objective` 로 이동만 했다" 였다. 본 설계는 그 재발을 막기 위해 다음
셋을 원칙으로 선언한다:

1. **Domain-first**: 트레잇/구조체의 모든 필드는 "사용자 애플리케이션의
   무엇을 표현하는가?" 한 문장으로 정당화 가능해야 한다. "튜닝해서
   수렴시켰다" 가 답이면 재제거 대상.
2. **Concrete-first**: 증거가 있는 도메인은 pair-prediction 하나 (SocioPatterns).
   일반 trait 은 두 번째 도메인 증거가 생길 때 추출. 지금은 `struct
   PairPredictionObjective` 한 개만 추가.
3. **No new scheduling knob**: Phase 8 에서 제거된 `auto_weather_every_ticks`
   계열을 재도입하지 않는다. 평가 시점은 사용자가 결정 (`engine.recognize_entities`
   와 동일한 패턴).

목표 축소: 본 API 가 튜닝하는 값은 **`PlasticityConfig.learning_rate` 하나**.
`weight_decay` 는 audit 291-293 에서 "non-default 증거 없음, 내부 const 후보"
로 이미 분류 — objective 루프에 끼워넣지 않는다.

중요한 수정 (2026-04-18 벤치 반영): 첫 초안은 pair ranking 기준을
`activity` 로 가정했지만, 이는 현재 엔진 구현과 맞지 않는다.
`activity` 는 관계 touch 와 decay 로 결정되고, Hebbian plasticity 는
`weight` slot 만 갱신한다. 새 suitability test 는 `activity` ranking 이
`learning_rate` 변화에 불변임을 확인했다. 따라서 본 문서는 Phase 9
objective 의 ranking 신호를 **`strength = activity + weight`** 로 바꾼다.
`strength` 는 prediction task 에 필요한 최근성(`activity`)과 plasticity 가
실제로 조정하는 장기 강화(`weight`)를 함께 담는다.

## 1. 각 필드의 domain-declaration 정당화

| 필드 | 도메인 질문 | tuning knob 이 아님을 보이는 답 |
|---|---|---|
| `k` | "UI/다운스트림이 한 prediction window 당 몇 개의 예측을 소비하는가?" | "추천 목록에 20개 띄운다" → `k=20`. 제품 스펙. |
| `horizon_batches` | "내 애플리케이션이 몇 batch 앞을 예측해야 하는가?" | "다음 수업 블록 1개" → `horizon=1 block 의 batch 수`. 프로덕트 정책. |
| `recall_weight` (λ) | "precision 대비 coverage 를 얼마나 중시하는가?" | 보안 경보 → 높음, UX 랭킹 → 낮음. product policy. |
| `kind` | "어느 `InfluenceKind` 의 plasticity 를 튜닝하는가?" | 도메인이 여러 kind 를 쓰면 kind 별로 objective 선언. |

하나라도 "도메인 문장" 이 안 나오면 그 use case 는 objective 를 쓰면 안 된다
— 기존처럼 `PlasticityConfig.learning_rate` 를 고정값으로 설정. 문서에서
이 점을 명시한다.

## 2. 평가 스케줄 — 사용자 주도

세 가지 후보 중:

| 후보 | 평가 |
|---|---|
| (1) 매 tick 자동 평가 | 비용 문제 + 새 스케줄 knob 필요 → **기각** |
| (2) 사용자가 `evaluate()` 호출 | 엔진 knob 0개, 기존 `recognize_entities` 패턴과 일관 → **채택** |
| (3) horizon window 가 만료되면 엔진이 자동 fire | 엔진이 snapshot 과 예측 로그를 보유 → state 증가 → 기각 |

채택 이유:
- Phase 8 의 "no auto-schedule" 방향을 깨지 않음
- 스트림의 자연스러운 cadence (블록/일/세션) 를 사용자가 이미 알고 있음
- trade-off: 사용자가 호출을 잊을 수 있음 — `recognize_entities` 와 동일한
  trade-off 를 이미 수용한 전례가 있으므로 감당 가능

## 3. `PairPredictionObjective` 구체 스펙

```rust
// crates/graph-engine/src/plasticity/objective.rs (신규)

use graph_core::{Change, ChangeId, InfluenceKindId, LocusId};
use graph_world::World;

/// Domain declaration: pair-prediction 형 plasticity 목표.
///
/// 본 struct 의 모든 필드는 사용자 애플리케이션의 구체적 질문에
/// 대응해야 한다 (§1 참조). 튜닝해서 수렴시키는 값이 있다면 그
/// use case 는 objective 를 쓰면 안 된다.
#[derive(Debug, Clone, Copy)]
pub struct PairPredictionObjective {
    /// 튜닝 대상이 되는 InfluenceKind. 도메인이 여러 kind 를 쓰면
    /// kind 별로 objective 인스턴스를 선언한다.
    pub kind: InfluenceKindId,
    /// Top-K — 다운스트림이 한 window 당 소비하는 예측 수.
    pub k: usize,
    /// Prediction horizon — 예측이 몇 batch 앞까지 유효해야 하는가.
    pub horizon_batches: u64,
    /// λ — `(1 − precision@K) + λ · (1 − recall)` 의 recall 가중치.
    /// `0.0` = precision-only, `1.0` = 대등.
    pub recall_weight: f32,
}

impl PairPredictionObjective {
    /// 현재 world 상태에서 `strength = activity + weight` 내림차순으로
    /// symmetric pair 를 랭크. `self.kind` 인 relationship 만 고려.
    /// hot memory 만 훑음 (cold relationship 은 제외 — 예측 공간은 hot).
    ///
    /// 이유:
    /// - `activity` 단독은 plasticity knob (`learning_rate`) 에 반응하지 않음
    /// - `weight` 단독은 최근 접촉성(recentness)을 놓칠 수 있음
    /// - `strength` 는 둘을 결합해 현재 엔진의 실제 동학과 정렬됨
    pub fn rank(&self, world: &World) -> Vec<((LocusId, LocusId), f32)> { /* … */ }

    /// 예측 리스트를 (from_batch, to_batch] 범위의 실제 관측
    /// Change 들과 대조해 점수 계산.
    ///
    /// `observed` 는 보통 `world.changes_to_relationship_in_range` /
    /// `changes_in_range` 로 얻은 iterator.
    pub fn score<'a, I: IntoIterator<Item = &'a Change>>(
        &self,
        predictions: &[(LocusId, LocusId)],
        observed: I,
        all_observed_pairs: &std::collections::HashSet<(LocusId, LocusId)>,
    ) -> PlasticityObservation { /* … */ }
}

/// 한 window 의 측정 결과. `Learnable::step` 의 observation 으로 소비됨.
#[derive(Debug, Clone, Copy)]
pub struct PlasticityObservation {
    /// `(1 − precision@K) + λ · (1 − recall)`
    pub loss: f32,
    pub precision_at_k: f32,
    pub recall: f32,
    pub k_used: usize,
    pub window_batches: u64,
}
```

`rank` 는 hot relationship 만 스캔하므로 cold-demoted pair 는 예측 공간에서
빠진다 — 이는 의도된 동작 (promotion 은 명시적이고 objective 도 그 선택을
따름). 또한 `rank` 의 점수는 `Relationship::strength()` 와 동일해야 한다.
즉, 사실상:

```rust
score = rel.activity() + rel.weight();
```

로 해석한다.

## 4. `Learnable` 연결

이미 존재하는 `regime/adaptive.rs::PerKindLearnable<L>` 프레임워크에
새 `impl Learnable` 하나를 추가하면 끝 — atomic state / register / clamp
보일러플레이트 전부 재사용.

```rust
// crates/graph-engine/src/plasticity/learner.rs (신규)

use crate::regime::Learnable;
use super::objective::PlasticityObservation;

/// `PlasticityConfig.learning_rate` 에 곱해질 per-kind 스케일.
/// `effective_learning_rate = config.learning_rate * learner.current(kind)`
pub struct PlasticityScale;

const MIN_SCALE: f32 = 0.1;
const MAX_SCALE: f32 = 3.0;
/// 1 window 당 스케일 변경 한도 — trust region 개념.
const STEP_BOUND: f32 = 1.2;

impl Learnable for PlasticityScale {
    type Observation = PlasticityObservation;
    fn initial() -> f32 { 1.0 }
    fn clamp_range() -> (f32, f32) { (MIN_SCALE, MAX_SCALE) }
    fn step(current: f32, obs: PlasticityObservation) -> f32 {
        // §5 open question 참조: step rule 은 설계 미확정.
        // 플레이스홀더 규칙: loss 가 0.5 를 넘으면 반 수축,
        // 0.1 미만이면 1.1 배 확장. "현재 loss 신호" 만 사용 —
        // Learnable 이 stateless 하므로 이전 loss 와 비교 불가.
        let bump = if obs.loss > 0.5 { 0.5 }
                   else if obs.loss < 0.1 { 1.1 }
                   else { 1.0 };
        (current * bump).clamp(current / STEP_BOUND, current * STEP_BOUND)
    }
}

pub type PlasticityLearners =
    crate::regime::adaptive::PerKindLearnable<PlasticityScale>;
```

**엔진 wiring**: `PlasticityConfig` 자체는 건드리지 않는다. 엔진 배치 루프
step 5 (Hebbian 업데이트) 에서 `learning_rate` 를 읽을 때:

```rust
// 현재:
let dw = config.learning_rate * pre * post;
// 제안:
let scale = learners.current(kind);    // 미등록 kind → 1.0
let dw = config.learning_rate * scale * pre * post;
```

Objective 를 쓰지 않는 사용자는 `PlasticityLearners::new()` 호출 안 하고
넘어가면 scale=1.0 — 기존 동작과 완전 동일 (Principle 1, opt-in).

## 5. 전체 end-to-end 패턴 (pseudocode)

```rust
// 1회 선언 — product spec 에서 도출된 상수
let obj = PairPredictionObjective {
    kind: CO_ATTEND,
    k: 20,                 // UI 상 top-20 을 보여준다
    horizon_batches: 3,    // "다음 블록" = 3 batch
    recall_weight: 0.5,    // 커버리지/정확도 balance
};
let mut learners = PlasticityLearners::new();
learners.register(CO_ATTEND);

// 사용자 루프
loop {
    // (a) 예측 스냅샷 — horizon window 시작
    let preds: Vec<_> = obj.rank(simulation.world())
        .into_iter().take(obj.k).map(|(k, _)| k).collect();
    let window_start = simulation.current_batch();

    // (b) horizon 만큼 진행
    for _ in 0..obj.horizon_batches {
        simulation.tick(next_stimuli());
    }

    // (c) window 내 관측된 pair 집합 수집
    let observed_changes = simulation.world()
        .changes_in_range(window_start, simulation.current_batch());
    let observed_pairs = extract_pairs(observed_changes, CO_ATTEND);

    // (d) 점수 계산 및 learner 업데이트
    let obs = obj.score(&preds, observed_changes, &observed_pairs);
    learners.observe(CO_ATTEND, obs);

    // 다음 tick 부터 effective learning_rate 에 scale 적용됨
}
```

엔진 내부는 `learners.current(kind)` 를 배치 루프 step 5 에서 읽기만
하면 끝. 추가 state 없음.

## 6. 새로 추가되는 표면

| 위치 | 추가 |
|---|---|
| `graph-engine::plasticity::objective` | `PairPredictionObjective` struct (필드 4개), `PlasticityObservation` struct (필드 5개) |
| `graph-engine::plasticity::learner` | `PlasticityScale` ZST impl Learnable, `PlasticityLearners` 타입 별칭 |
| 엔진 배치 루프 | Hebbian 업데이트 한 줄 (`* learners.current(kind)`). `PlasticityLearners` 를 `Engine::tick` 에 optional parameter 로 전달 |
| `InfluenceKindConfig` / `PlasticityConfig` | **변경 없음** |
| `SimulationBuilder` | 옵션: `with_plasticity_learners(learners)` 메서드 추가 고려 (결정 보류 §7) |

**총 필드 증가**: 9 개 (4 + 5). 이 중 observation 필드 5개는 **reporting** 용도
이므로 knob 이 아니다. knob 에 해당하는 것은 objective 필드 4개뿐 —
각각 §1 에서 domain-declaration 으로 정당화.

`PlasticityConfig` 는 3필드 그대로 (Phase 3 모습 유지). knob 총 surface 는
4개 증가하지만, 전부 opt-in (objective 를 선언한 사용자만 추가 부담).

## 7. 기각된 대안

1. **Generic `trait PlasticityObjective`** — 가상의 2번째 도메인을 위한
   slot 이 knob 가 될 위험. 증거 (pair-prediction) 하나만 있으므로 concrete
   struct 로 시작. 2번째 도메인 증거가 생길 때 trait 추출.
2. **엔진 자동 스케줄** — Phase 8 의 `auto_*_every_ticks` 제거 방향 역행.
3. **`weight_decay` 공동 튜닝** — audit 에서 "non-default 증거 없음" 이
   이미 확인. Objective 가 `learning_rate` scale 에서 유효성을 입증한
   **후** 별도 probe 로 확장 여부 결정.
4. **`weight`-only ranking** — plasticity 민감도는 높지만 최근 접촉성이 약해
   pair prediction task 의 "다음 window" 의미를 놓칠 수 있음. 현재 엔진에선
   `strength` 가 더 균형 잡힌 기본값.
5. **Gradient-based 튜너** — 스칼라 1개 / 드문 평가 / 노이즈 큰 측정.
   gradient-free 가 충분하고 단순.
6. **`PlasticityConfig.learning_rate: Option<f32>` 방식** — Phase 9
   scouting 에서 이미 롤백된 형태. 같은 실패 (hard-coded 대체값) 재발.
7. **`SubscriptionStore` 기반 자동 평가 fire** — 구독 시스템은 change
   스트림에 대한 것이지 objective 측정에 대한 것이 아님. layer 섞임.

## 8. Open questions (커밋 전 해결 필요)

1. **Step rule — stateless Learnable 의 한계**. `Learnable::step(current,
   obs)` 은 `prev_loss` 를 볼 수 없어 "loss 가 감소하면 direction 유지"
   류의 규칙을 쓸 수 없다. 해결안:
   - (a) `PlasticityObservation` 에 `prev_loss: Option<f32>` 추가 (사용자
     책임으로 지난 loss 를 넣도록) — 사용자 부담 증가
   - (b) `PerKindLearnable` 에 per-kind 보조 state 슬롯 도입 (프레임워크
     확장) — 범용성 ↑ 지만 framework churn
2. **`strength` 가 기본이어야 하는가?** 현재 증거는 `activity` 가 Phase 9
   objective 에 부적합하다는 것까지는 확정했고, `strength` 는 prediction
   품질과 plasticity 민감도를 함께 담는 실용적 기본값이다. 다만 장기적으로
   `weight` / `strength` / domain-specific scorer 를 전략 enum 으로 노출할지
   여부는 두 번째 도메인 증거가 생긴 뒤 재검토할 수 있다.

## 9. Success criteria (수정본)

Phase 9 는 아래 세 축을 모두 통과할 때만 "revived" 로 본다.

1. **Prediction suitability**: larger-profile stream 에서 top-K precision/lift 가
   random baseline 을 의미 있게 상회해야 한다.
2. **Plasticity sensitivity**: 같은 stream / seed 에서 `learning_rate` 변화가
   objective ranking 을 실제로 바꿔야 한다. `activity` ranking 은 이 조건을
   만족하지 못하므로 제외.
3. **Runtime cost**: end-to-end evaluation (`train -> rank -> score`) 비용이
   tuning loop 에서 감당 가능한 수준이어야 한다. `docs/phase9-benchmarks.md`
   기준 `xlarge/strength` 가 약 `0.42s` 수준이므로 현재는 통과.
   - (c) 현재 plasholder (loss 구간별 고정 배율) 유지 — 단순하지만 국소
     해 탈출 못 할 수 있음
   → §9 구현 차수 전 결정 필요. 직관상 (a) + placeholder 조합이 최소 변경.

2. **`rank` 의 비용**. 모든 hot relationship 스캔. SocioPatterns 규모
   (686 pair) 에서는 무시 가능, 대규모 (100k+) 스트림에서는 per-window
   오버헤드가 의미 있음. 완화책: `graph-query::filter::relationships_of_kind`
   를 이미 쓰므로 kind 필터는 O(n_kind). Top-K heap 도입 고려.

3. **Multi-kind objective**. 두 kind 가 같은 pair 에 기여하면 현 shape 은
   각 kind 별로 독립 평가. 실제 사례 아직 없음 — 증거 생기면 extend.

4. **Crate 위치**. Objective 가 engine 내부인지 query 쪽인지 경계 애매.
   본 초안은 `graph-engine` 밑 — 엔진 피드백 루프의 일부이므로. 다른
   선택지: `graph-query` (랭킹/스코어링이 read-only 이므로). 결정 보류.

5. **`SimulationBuilder::with_plasticity_learners`** 를 넣을 것인가, 아니면
   `Simulation::set_plasticity_learners(...)` 런타임 설정으로 할 것인가.
   빌더 포함이 opt-in 명시성 측면에서 선호 — 최종 결정 구현 단계에서.

## 9. 구현 차수 제안

1. **P1 — API skeleton**: `objective.rs` + `learner.rs` 추가. 엔진 wiring
   (Hebbian 한 줄) 는 `learners: Option<&PlasticityLearners>` 로 받아
   None 이면 무변경. 테스트: 기존 810+ 통과 유지 + `PlasticityScale`
   unit tests (`adaptive.rs` 의 7 tests 와 대칭 구조).
2. **P2 — SocioPatterns 통합 검증**: `sociopatterns.rs` 에 5번째 테스트
   `plasticity_auto_scale_beats_fixed` 추가. 동일 스트림, 같은 seed 에서
   objective-driven scale 이 고정 learning_rate 대비 precision@20 을
   개선하는지 (또는 최소한 나빠지지 않는지) 확인. 개선이 없으면 설계
   재검토 — objective 자체가 증거 없는 장식이 됨.
3. **P3 — Step rule 개선**: Open question #1 의 결정에 따라 stateful
   variant 또는 prev_loss carrying observation 적용. P2 의 경쟁력 결과가
   placeholder 규칙으로 이미 충분하면 P3 은 스킵 가능.

각 차수는 독립 커밋. P2 가 증거를 주지 못하면 P1 은 **immediate 롤백**
— Phase 9 scouting 과 동일한 기준 (증거 없는 knob 는 제거).

## 10. 성공/실패 기준

- **성공**: P2 테스트에서 objective-driven scale 이 precision@20 을
  유의미하게 (≥ 0.05 절대값) 개선 또는 noise-degraded variant 에서
  fixed 대비 회복 속도를 단축. `PlasticityConfig.learning_rate` 를
  공격적으로 잘못 설정한 출발점에서 합리적 값으로 수렴.
- **실패 (= 롤백)**: 차이가 측정 불가능하거나 오히려 악화. 이 경우
  Phase 9 reopen condition 을 다시 "false negative" 로 기록하고,
  진짜 문제는 API shape 이 아니라 supervised metric 자체가 engine
  parameter 에 무감각하다는 것 — 다른 시그널 탐색.
