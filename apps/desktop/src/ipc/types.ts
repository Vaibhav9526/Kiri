import { z } from 'zod';

export const projectSummarySchema = z.object({
  id: z.string().uuid(),
  title: z.string().min(1),
  path: z.string(),
  updatedAt: z.string(),
  missing: z.boolean(),
});
export type ProjectSummary = z.infer<typeof projectSummarySchema>;
export const jobProgressSchema = z.object({
  jobId: z.string().uuid(),
  state: z.enum(['queued', 'running', 'succeeded', 'failed', 'cancelled']),
  completed: z.number().int().nonnegative(),
  total: z.number().int().nonnegative().nullable().optional(),
  message: z.string(),
  updatedAt: z.string(),
});
export type JobProgress = z.infer<typeof jobProgressSchema>;
