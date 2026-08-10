import type { DmnEditorDocument } from '../../generated/editor-protocol';

/** Representative decision table shared by the DMN UI tests. */
export function sampleDmnDocument(): DmnEditorDocument {
  return {
    schemaVersion: '1.0',
    model: {
      id: 'leaveDefinitions',
      name: 'Leave definitions',
      namespace: 'https://flowable.org/dmn',
      decisions: [
        {
          id: 'leaveDecision',
          name: 'Leave approval',
          decisionTable: {
            id: 'leaveTable',
            hitPolicy: 'FIRST',
            inputs: [
              {
                id: 'inputDays',
                label: 'Leave days',
                inputNumber: 1,
                inputExpression: {
                  id: 'inputDaysExpression',
                  text: 'leaveDays',
                  typeRef: 'integer',
                },
              },
              {
                id: 'inputRole',
                label: 'Role',
                inputNumber: 2,
                inputExpression: {
                  id: 'inputRoleExpression',
                  text: 'role',
                  typeRef: 'string',
                },
              },
            ],
            outputs: [
              {
                id: 'outputStatus',
                label: 'Status',
                name: 'status',
                outputNumber: 1,
                typeRef: 'string',
              },
              {
                id: 'outputReason',
                name: 'reason',
                outputNumber: 2,
                typeRef: 'string',
              },
            ],
            rules: [
              {
                id: 'ruleManager',
                ruleNumber: 1,
                inputEntries: [
                  { id: 'ruleManagerDays', text: '> 5' },
                  { id: 'ruleManagerRole', text: '"manager"' },
                ],
                outputEntries: [
                  { id: 'ruleManagerStatus', text: '"APPROVED"', typeRef: 'string' },
                  { id: 'ruleManagerReason', text: '"Senior staff"', typeRef: 'string' },
                ],
              },
              {
                id: 'ruleFallback',
                ruleNumber: 2,
                inputEntries: [
                  { id: 'ruleFallbackDays', text: '-' },
                  { id: 'ruleFallbackRole', text: null },
                ],
                outputEntries: [
                  { id: 'ruleFallbackStatus', text: '"REVIEW"', typeRef: 'string' },
                  { id: 'ruleFallbackReason', text: '"Manual check"', typeRef: 'string' },
                ],
              },
            ],
          },
        },
      ],
    },
  };
}
