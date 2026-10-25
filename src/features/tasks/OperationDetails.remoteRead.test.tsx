import {render,screen} from '@testing-library/react';
import {expect,it,vi} from 'vitest';
import {OperationDetails} from './OperationDetails';
import type {ActionOperationResult,OperationSnapshot} from '../../contracts';
import fixture from '../../../fixtures/contracts/remote-actions-wire.json';
it('labels queued authenticated reads honestly and never renders fetch-only evidence or absence advice',()=>{
 const operation=fixture.result.operation as unknown as OperationSnapshot;
 render(<OperationDetails operation={operation} onCancel={vi.fn()} result={{operation,fetch:{prune:false,remotes:[]}} as ActionOperationResult}/>);
 expect(screen.getByRole('article',{name:'Read remote references task'})).toBeInTheDocument();expect(screen.queryByRole('list',{name:'Fetch remote results'})).not.toBeInTheDocument();expect(screen.queryByText(/Per-remote fetch evidence/)).not.toBeInTheDocument();expect(screen.queryByText('Fetch remotes')).not.toBeInTheDocument();expect(screen.getByText('No changes applied')).toBeInTheDocument();
});
