package org.flowable.rust.contract;

import java.util.Collection;
import java.util.List;
import java.util.Set;

import org.flowable.common.engine.api.delegate.event.FlowableEngineEventType;
import org.flowable.common.engine.api.delegate.event.FlowableEvent;
import org.flowable.common.engine.api.delegate.event.FlowableEventListener;
import org.flowable.common.engine.api.delegate.event.FlowableEventType;

public final class TransactionRecordingJobEventListener implements FlowableEventListener {

    private final String label;
    private final String onTransaction;
    private final boolean fatal;
    private final List<String> phases;

    public TransactionRecordingJobEventListener(
            String label,
            String onTransaction,
            boolean fatal,
            List<String> phases) {
        this.label = label;
        this.onTransaction = onTransaction;
        this.fatal = fatal;
        this.phases = phases;
    }

    @Override
    public void onEvent(FlowableEvent event) {
        phases.add(label);
        if (fatal) {
            throw new IllegalStateException("fatal " + label + " job event listener");
        }
    }

    @Override
    public boolean isFailOnException() {
        return fatal;
    }

    @Override
    public boolean isFireOnTransactionLifecycleEvent() {
        return onTransaction != null;
    }

    @Override
    public String getOnTransaction() {
        return onTransaction;
    }

    @Override
    public Collection<? extends FlowableEventType> getTypes() {
        return Set.of(FlowableEngineEventType.JOB_CANCELED);
    }
}
