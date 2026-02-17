use super::*;

pub(super) async fn seed_onboarding_conversation(
    repository: &dyn AgentCoreRepository,
    new_agent: &AgentRecord,
    all_agents: &[AgentRecord],
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
        enqueue_agent_to_agent_message(
            repository,
            new_agent,
            peer,
            format!(
                "Hi {}, I'm {}. Can we cooperate on {}?",
                peer.name, new_agent.name, topic
            ),
        )
        .await?;
        return Ok(1);
    }

    let trio = choose_distinct_agents(&peers, 2);
    if trio.len() < 2 {
        return Ok(0);
    }
    let left = &trio[0];
    let right = &trio[1];

    enqueue_agent_to_agent_message(
        repository,
        new_agent,
        left,
        format!(
            "Hi {}, I'm {}. Let's cooperate on {}.",
            left.name, new_agent.name, topic
        ),
    )
    .await?;
    enqueue_agent_to_agent_message(
        repository,
        left,
        right,
        format!(
            "{} just joined us. Can we coordinate this together?",
            new_agent.name
        ),
    )
    .await?;
    enqueue_agent_to_agent_message(
        repository,
        right,
        new_agent,
        format!(
            "Welcome {}, I'm in. Let's support each other on {}.",
            new_agent.name, topic
        ),
    )
    .await?;
    Ok(3)
}

pub(super) async fn seed_random_conversation(
    repository: &dyn AgentCoreRepository,
    all_agents: &[AgentRecord],
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
        enqueue_agent_to_agent_message(
            repository,
            first,
            second,
            format!(
                "{}, quick sync: can we cooperate on {}?",
                second.name, topic
            ),
        )
        .await?;

        if random_bool(70) {
            enqueue_agent_to_agent_message(
                repository,
                second,
                first,
                format!(
                    "Yes {}, I can support that. Thanks for starting the chat.",
                    first.name
                ),
            )
            .await?;
            return Ok(2);
        }
        return Ok(1);
    }

    let a = &participants[0];
    let b = &participants[1];
    let c = &participants[2];

    enqueue_agent_to_agent_message(
        repository,
        a,
        b,
        format!(
            "{}, let's coordinate {} and keep everyone aligned.",
            b.name, topic
        ),
    )
    .await?;
    enqueue_agent_to_agent_message(
        repository,
        b,
        c,
        format!(
            "{}, joining you both on {}. I can help with support.",
            c.name, topic
        ),
    )
    .await?;
    enqueue_agent_to_agent_message(
        repository,
        c,
        a,
        format!(
            "{}, agreed. Let's cooperate and share updates every step.",
            a.name
        ),
    )
    .await?;
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
