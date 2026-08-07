package org.flowable.rust.contract;

import java.util.ArrayList;
import java.util.Collection;
import java.util.EnumSet;
import java.util.List;

import org.flowable.common.engine.api.delegate.event.FlowableEngineEventType;
import org.flowable.common.engine.api.delegate.event.FlowableEvent;
import org.flowable.common.engine.api.delegate.event.FlowableEventListener;
import org.flowable.common.engine.api.delegate.event.FlowableEventType;

public final class RecordingJobEventListener implements FlowableEventListener {

    private final List<String> events = new ArrayList<>();

    @Override
    public void onEvent(FlowableEvent event) {
        events.add(event.getType().name());
    }

    @Override
    public boolean isFailOnException() {
        return true;
    }

    @Override
    public boolean isFireOnTransactionLifecycleEvent() {
        return false;
    }

    @Override
    public String getOnTransaction() {
        return null;
    }

    @Override
    public Collection<? extends FlowableEventType> getTypes() {
        return EnumSet.of(
                FlowableEngineEventType.ENTITY_UPDATED,
                FlowableEngineEventType.JOB_EXECUTION_FAILURE,
                FlowableEngineEventType.JOB_EXECUTION_SUCCESS,
                FlowableEngineEventType.JOB_MOVED_TO_DEADLETTER,
                FlowableEngineEventType.JOB_RETRIES_DECREMENTED);
    }

    public List<String> events() {
        return List.copyOf(events);
    }
}
