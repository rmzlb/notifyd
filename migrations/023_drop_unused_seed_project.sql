-- Migration 005 seeded a demo project ('craie', ten French e-commerce
-- templates) into every installation. On a fresh instance it is noise: a
-- project nobody asked for, a digest finding about its missing sender. Remove
-- it wherever it was never used; an instance that sent through it (or holds
-- subscribers under it) keeps everything.
DELETE FROM templates
 WHERE project_id = 'craie'
   AND NOT EXISTS (SELECT 1 FROM jobs WHERE project_id = 'craie')
   AND NOT EXISTS (SELECT 1 FROM subscribers WHERE project_id = 'craie');

DELETE FROM projects
 WHERE id = 'craie'
   AND NOT EXISTS (SELECT 1 FROM jobs WHERE project_id = 'craie')
   AND NOT EXISTS (SELECT 1 FROM subscribers WHERE project_id = 'craie')
   AND NOT EXISTS (SELECT 1 FROM templates WHERE project_id = 'craie');
