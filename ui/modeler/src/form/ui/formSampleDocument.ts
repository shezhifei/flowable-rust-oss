import type { FormEditorDocument } from '../../generated/editor-protocol';

/** Representative form document shared by the form designer tests. */
export function sampleFormDocument(): FormEditorDocument {
  return {
    schemaVersion: '1.0',
    model: {
      key: 'leaveForm',
      name: 'Leave request',
      description: 'Request a leave period',
      outcomeVariableName: null,
      fields: [
        { fieldType: 'BaseField', id: 'employeeName', type: 'text', name: 'Employee name', required: true },
        {
          fieldType: 'OptionFormField',
          id: 'leaveType',
          type: 'dropdown',
          name: 'Leave type',
          options: [
            { id: 'vacation', name: 'Vacation' },
            { id: 'sick', name: 'Sick leave' },
          ],
        },
        { fieldType: 'BaseField', id: 'approved', type: 'boolean', name: 'Pre-approved' },
        {
          fieldType: 'Container',
          id: 'periodContainer',
          type: 'container',
          name: 'Period',
          fields: [
            [{ fieldType: 'BaseField', id: 'startDate', type: 'date', name: 'Start date' }],
            [{ fieldType: 'BaseField', id: 'endDate', type: 'date', name: 'End date' }],
          ],
        },
      ],
      outcomes: [{ id: 'submit', name: 'Submit request' }],
    },
  };
}
