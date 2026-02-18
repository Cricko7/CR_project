use super::*;

pub(super) async fn seed_onboarding_conversation(
    repository: &dyn AgentCoreRepository,
    new_agent: &AgentRecord,
    all_agents: &[AgentRecord],
    llm: Option<&dyn LlmPort>,
) -> anyhow::Result<u32> {
    let peers: Vec<AgentRecord> = all_agents
        .iter()
        .filter(|agent| agent.id != new_agent.id)
        .cloned()
        .collect();
    if peers.is_empty() {
        return Ok(0);
    }

    let topic = pick_random_topic();
    if peers.len() == 1 {
        let peer = &peers[0];
        let fallback = format!(
            "Hi {}, I'm {}. Can we cooperate on {}?",
            peer.name, new_agent.name, topic
        );
        let content = compose_seed_message(
            llm,
            new_agent,
            peer,
            topic,
            "Introduce yourself and propose cooperation in one concise sentence.",
            fallback,
        )
        .await;
        enqueue_agent_to_agent_message(repository, new_agent, peer, content).await?;
        return Ok(1);
    }

    let trio = choose_distinct_agents(&peers, 2);
    if trio.len() < 2 {
        return Ok(0);
    }
    let left = &trio[0];
    let right = &trio[1];

    let first_fallback = format!(
        "Hi {}, I'm {}. Let's cooperate on {}.",
        left.name, new_agent.name, topic
    );
    let first_content = compose_seed_message(
        llm,
        new_agent,
        left,
        topic,
        "Greet the peer and invite them to cooperate on the shared topic.",
        first_fallback,
    )
    .await;
    enqueue_agent_to_agent_message(repository, new_agent, left, first_content).await?;

    let second_fallback = format!(
        "{} just joined us. Can we coordinate this together?",
        new_agent.name
    );
    let second_content = compose_seed_message(
        llm,
        left,
        right,
        topic,
        "Notify the third agent that a new participant joined and ask for coordination.",
        second_fallback,
    )
    .await;
    enqueue_agent_to_agent_message(repository, left, right, second_content).await?;

    let third_fallback = format!(
        "Welcome {}, I'm in. Let's support each other on {}.",
        new_agent.name, topic
    );
    let third_content = compose_seed_message(
        llm,
        right,
        new_agent,
        topic,
        "Welcome the new participant and confirm readiness to support the group.",
        third_fallback,
    )
    .await;
    enqueue_agent_to_agent_message(repository, right, new_agent, third_content).await?;
    Ok(3)
}

pub(super) async fn seed_random_conversation(
    repository: &dyn AgentCoreRepository,
    all_agents: &[AgentRecord],
    llm: Option<&dyn LlmPort>,
) -> anyhow::Result<u32> {
    if all_agents.len() < 2 {
        return Ok(0);
    }

    let group_size = if all_agents.len() >= 3 && random_bool(55) {
        3
    } else {
        2
    };
    let participants = choose_distinct_agents(all_agents, group_size);
    if participants.len() < 2 {
        return Ok(0);
    }

    let topic = pick_random_topic();
    if participants.len() == 2 {
        let first = &participants[0];
        let second = &participants[1];
        let first_fallback = format!(
            "{}, quick sync: can we cooperate on {}?",
            second.name, topic
        );
        let first_content = compose_seed_message(
            llm,
            first,
            second,
            topic,
            "Start a quick sync and ask for cooperation on the topic.",
            first_fallback,
        )
        .await;
        enqueue_agent_to_agent_message(repository, first, second, first_content).await?;

        if random_bool(70) {
            let second_fallback = format!(
                "Yes {}, I can support that. Thanks for starting the chat.",
                first.name
            );
            let second_content = compose_seed_message(
                llm,
                second,
                first,
                topic,
                "Acknowledge the proposal and confirm support in one short sentence.",
                second_fallback,
            )
            .await;
            enqueue_agent_to_agent_message(repository, second, first, second_content).await?;
            return Ok(2);
        }
        return Ok(1);
    }

    let a = &participants[0];
    let b = &participants[1];
    let c = &participants[2];

    let first_fallback = format!(
        "{}, let's coordinate {} and keep everyone aligned.",
        b.name, topic
    );
    let first_content = compose_seed_message(
        llm,
        a,
        b,
        topic,
        "Ask for coordination and emphasize team alignment.",
        first_fallback,
    )
    .await;
    enqueue_agent_to_agent_message(repository, a, b, first_content).await?;

    let second_fallback = format!(
        "{}, joining you both on {}. I can help with support.",
        c.name, topic
    );
    let second_content = compose_seed_message(
        llm,
        b,
        c,
        topic,
        "Join the discussion and offer practical support.",
        second_fallback,
    )
    .await;
    enqueue_agent_to_agent_message(repository, b, c, second_content).await?;

    let third_fallback = format!(
        "{}, agreed. Let's cooperate and share updates every step.",
        a.name
    );
    let third_content = compose_seed_message(
        llm,
        c,
        a,
        topic,
        "Confirm agreement and suggest sharing updates while cooperating.",
        third_fallback,
    )
    .await;
    enqueue_agent_to_agent_message(repository, c, a, third_content).await?;
    Ok(3)
}

pub(super) async fn enqueue_agent_to_agent_message(
    repository: &dyn AgentCoreRepository,
    sender: &AgentRecord,
    receiver: &AgentRecord,
    content: String,
) -> anyhow::Result<()> {
    if sender.id == receiver.id {
        return Ok(());
    }

    repository
        .enqueue_message(&NewMessage {
            sender_type: "agent".to_owned(),
            sender_id: Some(sender.id),
            receiver_agent_id: receiver.id,
            content: trim_text(&content, CONVERSATION_MESSAGE_MAX_CHARS),
        })
        .await?;
    Ok(())
}

pub(super) fn choose_distinct_agents(agents: &[AgentRecord], count: usize) -> Vec<AgentRecord> {
    let mut pool: Vec<usize> = (0..agents.len()).collect();
    let target = count.min(pool.len());
    let mut selected = Vec::with_capacity(target);

    while selected.len() < target && !pool.is_empty() {
        let idx = random_index(pool.len());
        let source_idx = pool.swap_remove(idx);
        selected.push(agents[source_idx].clone());
    }

    selected
}

pub(super) fn pick_random_topic() -> &'static str {
    let idx = random_index(CONVERSATION_TOPICS.len());
    CONVERSATION_TOPICS[idx]
}

async fn compose_seed_message(
    llm: Option<&dyn LlmPort>,
    sender: &AgentRecord,
    receiver: &AgentRecord,
    topic: &str,
    intent: &str,
    fallback: String,
) -> String {
    let Some(llm) = llm else {
        return fallback;
    };

    let request = sim_backend::llm::LlmGenerateRequest {
        system_prompt: Some(format!(
            "You generate one short in-world chat message from one autonomous agent to another. \
             Return plain text only, no markdown, no quotes, max 220 chars."
        )),
        user_prompt: format!(
            "Sender: {} (personality: {}). Receiver: {} (personality: {}). Topic: {}. Intent: {}. \
             Produce exactly one concise message to the receiver.",
            sender.name,
            sender.personality_json,
            receiver.name,
            receiver.personality_json,
            topic,
            intent
        ),
        temperature: Some(0.7),
        max_output_tokens: Some(90),
    };

    match llm.generate(request).await {
        Ok(response) => {
            let text = trim_text(&response.text, CONVERSATION_MESSAGE_MAX_CHARS);
            tracing::debug!(
                sender_id = %sender.id,
                receiver_id = %receiver.id,
                model = %response.model,
                "generated inter-agent seed message via llm"
            );
            if text.is_empty() { fallback } else { text }
        }
        Err(error) => {
            tracing::warn!(
                sender_id = %sender.id,
                receiver_id = %receiver.id,
                error = %error,
                "failed to generate llm conversation message, using deterministic fallback"
            );
            fallback
        }
    }
}
