-- Craie project setup for notifyd
-- 10 email templates + project config

INSERT INTO projects (id, api_key, name, channels, created_at) 
VALUES (
    'craie', 
    encode(gen_random_bytes(32), 'hex'),
    'Craie — Textiles sur mesure', 
    '{email}',
    now()
)
ON CONFLICT (id) DO NOTHING;

-- ── Templates ──────────────────────────────────────────────────────

-- Cart abandoned
INSERT INTO templates (id, project_id, channel, subject, body, body_html) VALUES (
    'cart_abandoned', 'craie', 'email',
    'Votre panier vous attend chez Craie',
    'Bonjour, vous avez laissé des articles dans votre panier. Retrouvez-les ici : {{cart_url}}',
    '<table width="100%" cellpadding="0" cellspacing="0" style="max-width:600px;margin:0 auto;font-family:Inter,Helvetica,Arial,sans-serif;color:#111111;background-color:#FAF9F6"><tr><td style="padding:40px 30px;text-align:center"><h1 style="font-family:''Playfair Display'',Georgia,serif;font-size:28px;font-weight:400;margin:0 0 20px">Votre panier vous attend</h1><p style="font-size:15px;line-height:1.6;color:#57534e">Vous avez laissé des pièces dans votre panier. Elles sont encore disponibles — pour l''instant.</p><a href="{{cart_url}}" style="display:inline-block;margin:25px 0;padding:14px 32px;background:#111111;color:#FAF9F6;text-decoration:none;font-size:14px;letter-spacing:0.5px">REPRENDRE MON PANIER</a><p style="font-size:13px;color:#a8a29e;margin-top:30px">Craie — Textiles sur mesure</p></td></tr></table>'
) ON CONFLICT (project_id, id, channel) DO UPDATE SET subject=EXCLUDED.subject, body=EXCLUDED.body, body_html=EXCLUDED.body_html, updated_at=now();

-- Checkout incomplete
INSERT INTO templates (id, project_id, channel, subject, body, body_html) VALUES (
    'checkout_incomplete', 'craie', 'email',
    'Finalisez votre commande Craie',
    'Bonjour, votre commande est presque prête. Finalisez-la ici : {{checkout_url}}',
    '<table width="100%" cellpadding="0" cellspacing="0" style="max-width:600px;margin:0 auto;font-family:Inter,Helvetica,Arial,sans-serif;color:#111111;background-color:#FAF9F6"><tr><td style="padding:40px 30px;text-align:center"><h1 style="font-family:''Playfair Display'',Georgia,serif;font-size:28px;font-weight:400;margin:0 0 20px">Plus qu''une étape</h1><p style="font-size:15px;line-height:1.6;color:#57534e">Votre commande est presque prête. Il ne reste plus qu''à confirmer.</p><a href="{{checkout_url}}" style="display:inline-block;margin:25px 0;padding:14px 32px;background:#111111;color:#FAF9F6;text-decoration:none;font-size:14px;letter-spacing:0.5px">FINALISER MA COMMANDE</a><p style="font-size:13px;color:#a8a29e;margin-top:30px">Craie — Textiles sur mesure</p></td></tr></table>'
) ON CONFLICT (project_id, id, channel) DO UPDATE SET subject=EXCLUDED.subject, body=EXCLUDED.body, body_html=EXCLUDED.body_html, updated_at=now();

-- Order confirmed
INSERT INTO templates (id, project_id, channel, subject, body, body_html) VALUES (
    'order_confirmed', 'craie', 'email',
    'Commande confirmée — {{order_number}}',
    'Votre commande {{order_number}} est confirmée. Suivez-la ici : {{order_url}}',
    '<table width="100%" cellpadding="0" cellspacing="0" style="max-width:600px;margin:0 auto;font-family:Inter,Helvetica,Arial,sans-serif;color:#111111;background-color:#FAF9F6"><tr><td style="padding:40px 30px;text-align:center"><h1 style="font-family:''Playfair Display'',Georgia,serif;font-size:28px;font-weight:400;margin:0 0 20px">Commande confirmée</h1><p style="font-size:15px;line-height:1.6;color:#57534e">Merci pour votre commande <strong>{{order_number}}</strong>. Nous la préparons avec soin.</p><a href="{{order_url}}" style="display:inline-block;margin:25px 0;padding:14px 32px;background:#111111;color:#FAF9F6;text-decoration:none;font-size:14px;letter-spacing:0.5px">SUIVRE MA COMMANDE</a><p style="font-size:13px;color:#a8a29e;margin-top:30px">Craie — Textiles sur mesure</p></td></tr></table>'
) ON CONFLICT (project_id, id, channel) DO UPDATE SET subject=EXCLUDED.subject, body=EXCLUDED.body, body_html=EXCLUDED.body_html, updated_at=now();

-- Order shipped
INSERT INTO templates (id, project_id, channel, subject, body, body_html) VALUES (
    'order_shipped', 'craie', 'email',
    'Votre commande est en route — {{order_number}}',
    'Bonne nouvelle ! Votre commande {{order_number}} est en cours de livraison. Suivi : {{tracking_url}}',
    '<table width="100%" cellpadding="0" cellspacing="0" style="max-width:600px;margin:0 auto;font-family:Inter,Helvetica,Arial,sans-serif;color:#111111;background-color:#FAF9F6"><tr><td style="padding:40px 30px;text-align:center"><h1 style="font-family:''Playfair Display'',Georgia,serif;font-size:28px;font-weight:400;margin:0 0 20px">C''est en route !</h1><p style="font-size:15px;line-height:1.6;color:#57534e">Votre commande <strong>{{order_number}}</strong> a été expédiée.</p><a href="{{tracking_url}}" style="display:inline-block;margin:25px 0;padding:14px 32px;background:#111111;color:#FAF9F6;text-decoration:none;font-size:14px;letter-spacing:0.5px">SUIVRE MON COLIS</a><p style="font-size:13px;color:#a8a29e;margin-top:30px">Craie — Textiles sur mesure</p></td></tr></table>'
) ON CONFLICT (project_id, id, channel) DO UPDATE SET subject=EXCLUDED.subject, body=EXCLUDED.body, body_html=EXCLUDED.body_html, updated_at=now();

-- Order delivered
INSERT INTO templates (id, project_id, channel, subject, body, body_html) VALUES (
    'order_delivered', 'craie', 'email',
    'Commande livrée — {{order_number}}',
    'Votre commande {{order_number}} a été livrée. Nous espérons qu''elle vous plaît !',
    '<table width="100%" cellpadding="0" cellspacing="0" style="max-width:600px;margin:0 auto;font-family:Inter,Helvetica,Arial,sans-serif;color:#111111;background-color:#FAF9F6"><tr><td style="padding:40px 30px;text-align:center"><h1 style="font-family:''Playfair Display'',Georgia,serif;font-size:28px;font-weight:400;margin:0 0 20px">C''est livré !</h1><p style="font-size:15px;line-height:1.6;color:#57534e">Votre commande <strong>{{order_number}}</strong> a été livrée. Nous espérons que nos textiles vous raviront.</p><a href="{{review_url}}" style="display:inline-block;margin:25px 0;padding:14px 32px;background:#111111;color:#FAF9F6;text-decoration:none;font-size:14px;letter-spacing:0.5px">DONNER MON AVIS</a><p style="font-size:13px;color:#a8a29e;margin-top:30px">Craie — Textiles sur mesure</p></td></tr></table>'
) ON CONFLICT (project_id, id, channel) DO UPDATE SET subject=EXCLUDED.subject, body=EXCLUDED.body, body_html=EXCLUDED.body_html, updated_at=now();

-- Review request
INSERT INTO templates (id, project_id, channel, subject, body, body_html) VALUES (
    'review_request', 'craie', 'email',
    'Comment trouvez-vous vos textiles Craie ?',
    'Bonjour ! Vos textiles sont arrivés il y a quelques jours. Donnez-nous votre avis : {{review_url}}',
    '<table width="100%" cellpadding="0" cellspacing="0" style="max-width:600px;margin:0 auto;font-family:Inter,Helvetica,Arial,sans-serif;color:#111111;background-color:#FAF9F6"><tr><td style="padding:40px 30px;text-align:center"><h1 style="font-family:''Playfair Display'',Georgia,serif;font-size:28px;font-weight:400;margin:0 0 20px">Votre avis compte</h1><p style="font-size:15px;line-height:1.6;color:#57534e">Vos textiles Craie sont arrivés il y a quelques jours. Comment les trouvez-vous ?</p><a href="{{review_url}}" style="display:inline-block;margin:25px 0;padding:14px 32px;background:#111111;color:#FAF9F6;text-decoration:none;font-size:14px;letter-spacing:0.5px">DONNER MON AVIS</a><p style="font-size:13px;color:#a8a29e;margin-top:30px">Craie — Textiles sur mesure</p></td></tr></table>'
) ON CONFLICT (project_id, id, channel) DO UPDATE SET subject=EXCLUDED.subject, body=EXCLUDED.body, body_html=EXCLUDED.body_html, updated_at=now();

-- Quote ready
INSERT INTO templates (id, project_id, channel, subject, body, body_html) VALUES (
    'quote_ready', 'craie', 'email',
    'Votre devis Craie est prêt',
    'Votre devis est disponible. Consultez-le ici : {{quote_url}}',
    '<table width="100%" cellpadding="0" cellspacing="0" style="max-width:600px;margin:0 auto;font-family:Inter,Helvetica,Arial,sans-serif;color:#111111;background-color:#FAF9F6"><tr><td style="padding:40px 30px;text-align:center"><h1 style="font-family:''Playfair Display'',Georgia,serif;font-size:28px;font-weight:400;margin:0 0 20px">Votre devis est prêt</h1><p style="font-size:15px;line-height:1.6;color:#57534e">Nous avons préparé votre devis personnalisé. Il est disponible dans votre espace.</p><a href="{{quote_url}}" style="display:inline-block;margin:25px 0;padding:14px 32px;background:#111111;color:#FAF9F6;text-decoration:none;font-size:14px;letter-spacing:0.5px">VOIR MON DEVIS</a><p style="font-size:13px;color:#a8a29e;margin-top:30px">Craie — Textiles sur mesure</p></td></tr></table>'
) ON CONFLICT (project_id, id, channel) DO UPDATE SET subject=EXCLUDED.subject, body=EXCLUDED.body, body_html=EXCLUDED.body_html, updated_at=now();

-- Sample shipped
INSERT INTO templates (id, project_id, channel, subject, body, body_html) VALUES (
    'sample_shipped', 'craie', 'email',
    'Vos échantillons Craie sont en route',
    'Vos échantillons de tissus ont été expédiés. Suivi : {{tracking_url}}',
    '<table width="100%" cellpadding="0" cellspacing="0" style="max-width:600px;margin:0 auto;font-family:Inter,Helvetica,Arial,sans-serif;color:#111111;background-color:#FAF9F6"><tr><td style="padding:40px 30px;text-align:center"><h1 style="font-family:''Playfair Display'',Georgia,serif;font-size:28px;font-weight:400;margin:0 0 20px">Échantillons expédiés</h1><p style="font-size:15px;line-height:1.6;color:#57534e">Vos échantillons de tissus sont en chemin. Vous pourrez bientôt les toucher, les comparer, les vivre.</p><a href="{{tracking_url}}" style="display:inline-block;margin:25px 0;padding:14px 32px;background:#111111;color:#FAF9F6;text-decoration:none;font-size:14px;letter-spacing:0.5px">SUIVRE MES ÉCHANTILLONS</a><p style="font-size:13px;color:#a8a29e;margin-top:30px">Craie — Textiles sur mesure</p></td></tr></table>'
) ON CONFLICT (project_id, id, channel) DO UPDATE SET subject=EXCLUDED.subject, body=EXCLUDED.body, body_html=EXCLUDED.body_html, updated_at=now();

-- Welcome
INSERT INTO templates (id, project_id, channel, subject, body, body_html) VALUES (
    'welcome', 'craie', 'email',
    'Bienvenue chez Craie',
    'Bienvenue ! Découvrez nos collections : {{store_url}}',
    '<table width="100%" cellpadding="0" cellspacing="0" style="max-width:600px;margin:0 auto;font-family:Inter,Helvetica,Arial,sans-serif;color:#111111;background-color:#FAF9F6"><tr><td style="padding:40px 30px;text-align:center"><h1 style="font-family:''Playfair Display'',Georgia,serif;font-size:28px;font-weight:400;margin:0 0 20px">Bienvenue chez Craie</h1><p style="font-size:15px;line-height:1.6;color:#57534e">Merci de rejoindre Craie. Découvrez nos textiles fabriqués sur mesure, pensés pour durer.</p><a href="{{store_url}}" style="display:inline-block;margin:25px 0;padding:14px 32px;background:#111111;color:#FAF9F6;text-decoration:none;font-size:14px;letter-spacing:0.5px">DÉCOUVRIR LA COLLECTION</a><p style="font-size:13px;color:#a8a29e;margin-top:30px">Craie — Textiles sur mesure</p></td></tr></table>'
) ON CONFLICT (project_id, id, channel) DO UPDATE SET subject=EXCLUDED.subject, body=EXCLUDED.body, body_html=EXCLUDED.body_html, updated_at=now();

-- Back in stock
INSERT INTO templates (id, project_id, channel, subject, body, body_html) VALUES (
    'back_in_stock', 'craie', 'email',
    '{{product_name}} est de retour en stock',
    'Bonne nouvelle ! {{product_name}} est à nouveau disponible : {{product_url}}',
    '<table width="100%" cellpadding="0" cellspacing="0" style="max-width:600px;margin:0 auto;font-family:Inter,Helvetica,Arial,sans-serif;color:#111111;background-color:#FAF9F6"><tr><td style="padding:40px 30px;text-align:center"><h1 style="font-family:''Playfair Display'',Georgia,serif;font-size:28px;font-weight:400;margin:0 0 20px">De retour en stock</h1><p style="font-size:15px;line-height:1.6;color:#57534e"><strong>{{product_name}}</strong> est à nouveau disponible. Ne le manquez pas cette fois.</p><a href="{{product_url}}" style="display:inline-block;margin:25px 0;padding:14px 32px;background:#111111;color:#FAF9F6;text-decoration:none;font-size:14px;letter-spacing:0.5px">VOIR LE PRODUIT</a><p style="font-size:13px;color:#a8a29e;margin-top:30px">Craie — Textiles sur mesure</p></td></tr></table>'
) ON CONFLICT (project_id, id, channel) DO UPDATE SET subject=EXCLUDED.subject, body=EXCLUDED.body, body_html=EXCLUDED.body_html, updated_at=now();
