import { token as first, loads } from './first.js';
import { token as second } from '../shared/./state.mjs';
export default { same: first === second, loads };
