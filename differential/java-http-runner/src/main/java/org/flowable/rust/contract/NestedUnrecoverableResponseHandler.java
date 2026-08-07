package org.flowable.rust.contract;

import org.flowable.common.engine.api.FlowableException;
import org.flowable.common.engine.api.variable.VariableContainer;
import org.flowable.http.common.api.HttpResponse;
import org.flowable.http.common.api.delegate.HttpResponseHandler;
import org.flowable.job.api.FlowableUnrecoverableJobException;

public final class NestedUnrecoverableResponseHandler implements HttpResponseHandler {

    @Override
    public void handleHttpResponse(VariableContainer execution, HttpResponse httpResponse) {
        throw new FlowableException(
                "response handler wrapper failed",
                new FlowableUnrecoverableJobException(
                        "response payload cannot be safely processed"));
    }
}
